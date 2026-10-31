use super::*;
use crate::session::persistence::default_model_id;
use crate::session::storage::{StorageAdapter, strip_rewrite_gated};
use agent_client_protocol::{ImageContent, SessionId};
use xai_chat_state::interjection::InterjectionDisposition;
use xai_grok_sampling_types::{InterjectionIdentity, SourceOccurrence, SourceOccurrenceId};
use std::io::Write;

fn receipt(id: &str, text: &str, source: u8) -> InterjectionReceipt {
    InterjectionReceipt::new(
        InterjectionIdentity { id: id.into(), display_text: text.into() },
        vec![ImageContent::new("AA==", "image/png")],
        SourceOccurrence::Admitted { id: SourceOccurrenceId([source; 16]) },
    ).unwrap()
}
async fn fixture() -> (tempfile::TempDir, JsonlStorageAdapter, Info) {
    let dir = tempfile::tempdir().unwrap();
    let storage = JsonlStorageAdapter::with_root(dir.path().to_path_buf());
    let info = Info { id: SessionId::new("interjection-test"), cwd: "/fixture".into() };
    storage.init_session(&info, default_model_id()).await.unwrap();
    (dir, storage, info)
}

#[tokio::test]
async fn durable_retry_and_conflict_return_original_after_cold_restore() {
    let (_dir, storage, info) = fixture().await;
    let original = receipt("one", "original", 1);
    let (state, _) = storage.restore_interjections(&info).await.unwrap();
    assert!(matches!(storage.append_interjection_receipt(&info, &state, &original).await.unwrap(), InterjectionAppend::Appended(_)));
    let before = std::fs::read(storage.chat_file(&info)).unwrap();
    let (state, _) = storage.restore_interjections(&info).await.unwrap();
    match storage.append_interjection_receipt(&info, &state, &receipt("one", "original", 2)).await.unwrap() {
        InterjectionAppend::AlreadyPresent(saved) => { assert_eq!(saved.source(), original.source()); assert_eq!(saved.images(), original.images()); }
        _ => panic!("expected durable duplicate"),
    }
    match storage.append_interjection_receipt(&info, &state, &receipt("one", "different", 3)).await.unwrap() {
        InterjectionAppend::Conflict(saved) => assert_eq!(saved.identity(), original.identity()),
        _ => panic!("expected payload conflict"),
    }
    assert_eq!(std::fs::read(storage.chat_file(&info)).unwrap(), before);
    assert!(matches!(storage.append_interjection_receipt(&info, &state, &receipt("two", "original", 4)).await.unwrap(), InterjectionAppend::Appended(_)));
}

#[tokio::test]
async fn projection_is_one_record_and_rewrite_retains_original_source_and_images() {
    let (_dir, storage, info) = fixture().await;
    let original = receipt("one", "original", 1);
    let (state, _) = storage.restore_interjections(&info).await.unwrap();
    storage.append_interjection_receipt(&info, &state, &original).await.unwrap();
    let mut item = ConversationItem::interjection(xai_interjection_core::format_interjection("original".into()));
    if let ConversationItem::User(user) = &mut item {
        user.source = original.source();
        user.interjection = Some(original.identity().clone());
    }
    item.add_image("data:image/png;base64,AA==");
    let projected = original.with_disposition(InterjectionDisposition::Projected).unwrap();
    storage.persist_interjection_projection(&info, &state, InterjectionProjection::Projected { receipt: projected, item }).await.unwrap();
    assert_eq!(storage.read_chat_history_sync(storage.chat_file(&info), CHAT_FORMAT_VERSION).unwrap().len(), 1);
    storage.replace_chat_history(&info, &[ConversationItem::user("compacted")]).await.unwrap();
    strip_rewrite_gated(&storage, &info, &[]).await.unwrap();
    storage.replace_chat_history(&info, &[]).await.unwrap();
    let (_, restored) = storage.restore_interjections(&info).await.unwrap();
    let saved = restored.get("one").unwrap();
    assert_eq!(saved.source(), original.source());
    assert_eq!(saved.images(), original.images());
    assert_eq!(saved.identity(), original.identity());
    assert_eq!(saved.disposition(), InterjectionDisposition::Projected);
    assert!(storage.read_chat_history_sync(storage.chat_file(&info), CHAT_FORMAT_VERSION).unwrap().is_empty());
    assert!(matches!(storage.append_interjection_receipt(&info, &state, &original).await.unwrap(), InterjectionAppend::AlreadyPresent(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_loaded_writers_serialize_one_admission_and_return_the_winning_source() {
    let (_dir, storage, info) = fixture().await;
    let (first, _) = storage.restore_interjections(&info).await.unwrap();
    let (second, _) = storage.restore_interjections(&info).await.unwrap();
    let storage = Arc::new(storage);
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut jobs = Vec::new();
    for (state, source) in [(first, 1), (second, 2)] {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        let info = info.clone();
        jobs.push(tokio::spawn(async move {
            barrier.wait().await;
            storage.append_interjection_receipt(&info, &state, &receipt("one", "original", source)).await.unwrap()
        }));
    }
    let mut appended = Vec::new();
    let mut duplicates = Vec::new();
    for job in jobs {
        match job.await.unwrap() {
            InterjectionAppend::Appended(saved) => appended.push(saved),
            InterjectionAppend::AlreadyPresent(saved) => duplicates.push(saved),
            InterjectionAppend::Conflict(_) => panic!("identical concurrent payloads must not conflict"),
        }
    }
    assert_eq!(appended.len(), 1);
    assert_eq!(duplicates.len(), 1);
    assert_eq!(appended[0].source(), duplicates[0].source());
    let disk = std::fs::read(storage.chat_file(&info)).unwrap();
    assert_eq!(disk.iter().filter(|byte| **byte == b'\n').count(), 1);
    let (_, restored) = storage.restore_interjections(&info).await.unwrap();
    assert_eq!(restored.get("one").unwrap().source(), appended[0].source());
}

#[tokio::test]
async fn torn_tail_cannot_swallow_a_new_admission() {
    let (_dir, storage, info) = fixture().await;
    let (state, _) = storage.restore_interjections(&info).await.unwrap();
    let path = storage.chat_file(&info);
    std::fs::OpenOptions::new().append(true).create(true).open(&path).unwrap().write_all(b"{\"torn\":").unwrap();
    storage.append_interjection_receipt(&info, &state, &receipt("one", "original", 1)).await.unwrap();
    assert!(storage.restore_interjections(&info).await.unwrap().1.get("one").is_some());
}

#[tokio::test]
async fn session_binding_rejects_another_session_before_writing() {
    let (_dir, storage, info) = fixture().await;
    let (state, _) = storage.restore_interjections(&info).await.unwrap();
    let other = Info { id: SessionId::new("other"), cwd: info.cwd.clone() };
    assert!(matches!(storage.append_interjection_receipt(&other, &state, &receipt("one", "original", 1)).await, Err(InterjectionStorageError::WrongSession)));
}

#[tokio::test]
async fn retirement_survives_rewind_without_resurrection() {
    let (_dir, storage, info) = fixture().await;
    let original = receipt("one", "original", 1);
    let (state, _) = storage.restore_interjections(&info).await.unwrap();
    storage.append_interjection_receipt(&info, &state, &original).await.unwrap();
    storage.persist_interjection_projection(&info, &state, InterjectionProjection::Retired { receipt: original.with_disposition(InterjectionDisposition::Retired).unwrap() }).await.unwrap();
    storage.replace_chat_history(&info, &[]).await.unwrap();
    let (restored, _) = storage.restore_interjections(&info).await.unwrap();
    match storage.append_interjection_receipt(&info, &restored, &original).await.unwrap() {
        InterjectionAppend::AlreadyPresent(saved) => assert_eq!(saved.disposition(), InterjectionDisposition::Retired),
        _ => panic!("retired admission must not resurrect"),
    }
}

#[test]
fn append_error_distinguishes_before_write_from_failed_durable_ack() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history");
    let line = serde_json::to_vec(&ReceiptRecord { interjection_receipt: receipt("one", "original", 1) }).unwrap();
    let mut line = line;
    line.push(b'\n');
    let result = JsonlStorageAdapter::append_jsonl_line_sync_with(&path, line.clone(), AppendDurability::Durable, |_| Err(io::Error::other("sync failure")), || Ok(()));
    assert!(matches!(result, Err(AppendLineError::Committed(_))));
    assert_eq!(std::fs::read(&path).unwrap(), line);
    let missing = dir.path().join("missing").join("history");
    assert!(matches!(JsonlStorageAdapter::append_jsonl_line_sync_with(&missing, line, AppendDurability::Durable, |_| Ok(()), || Ok(())), Err(AppendLineError::NotCommitted(_))));
}

#[tokio::test]
async fn lost_durable_ack_retries_without_a_second_record() {
    let (_dir, storage, info) = fixture().await;
    let path = storage.chat_file(&info);
    let original = receipt("one", "original", 1);
    let mut line = serde_json::to_vec(&ReceiptRecord { interjection_receipt: original.clone() }).unwrap();
    line.push(b'\n');
    assert!(matches!(JsonlStorageAdapter::append_jsonl_line_sync_with(&path, line.clone(), AppendDurability::Durable, |_| Err(io::Error::other("lost sync ack")), || Ok(())), Err(AppendLineError::Committed(_))));
    let (restored, _) = storage.restore_interjections(&info).await.unwrap();
    assert!(matches!(storage.append_interjection_receipt(&info, &restored, &original).await.unwrap(), InterjectionAppend::AlreadyPresent(_)));
    assert_eq!(std::fs::read(path).unwrap(), line);
}

#[test]
fn long_lived_session_retains_every_identity_and_large_original_payload() {
    let mut index = InterjectionIndex::default();
    for number in 0_u128..5000 {
        let admitted = InterjectionReceipt::new(
            InterjectionIdentity { id: number.to_string(), display_text: "body".into() },
            Vec::new(), SourceOccurrence::Admitted { id: SourceOccurrenceId((number + 1).to_be_bytes()) },
        ).unwrap();
        index.restore(admitted).unwrap();
    }
    let long_id = "x".repeat(2048);
    let body = "b".repeat(17 * 1024 * 1024);
    for number in 0..4 {
        let id = format!("{long_id}-{number}");
        let admitted = InterjectionReceipt::new(
            InterjectionIdentity { id: id.clone(), display_text: body.clone() }, Vec::new(),
            SourceOccurrence::Admitted { id: SourceOccurrenceId((5001_u128 + number).to_be_bytes()) },
        ).unwrap();
        let wire = serde_json::to_vec(&admitted).unwrap();
        let restored: InterjectionReceipt = serde_json::from_slice(&wire).unwrap();
        index.restore(restored).unwrap();
        assert_eq!(index.get(&id).unwrap().identity().display_text, body);
    }
    assert_eq!(index.receipts().count(), 5004);
    for number in 0..5000 {
        assert_eq!(index.get(&number.to_string()).unwrap().identity().display_text, "body");
    }
    assert_eq!(index.get("0").unwrap().source(), SourceOccurrence::Admitted { id: SourceOccurrenceId(1_u128.to_be_bytes()) });
}

#[test]
fn unadmitted_and_legacy_sources_are_not_admission_receipts() {
    for source in [SourceOccurrence::Unadmitted, SourceOccurrence::LegacyNormalized { id: SourceOccurrenceId([1; 16]) }] {
        assert!(matches!(InterjectionReceipt::new(InterjectionIdentity { id: "one".into(), display_text: "original".into() }, vec![], source), Err(ReceiptError::InvalidSource)));
    }
}

#[tokio::test]
async fn partial_write_error_preserves_prefix_and_cold_retry_retains_one_complete_admission() {
    let (_dir, storage, info) = fixture().await;
    let path = storage.chat_file(&info);
    let original = receipt("one", "original", 1);
    let mut line = serde_json::to_vec(&ReceiptRecord { interjection_receipt: original.clone() }).unwrap();
    line.push(b'\n');
    let prefix_len = line.len() / 2;
    {
        let _lock = JsonlStorageAdapter::lock_append(&path).unwrap();
        let failed = JsonlStorageAdapter::append_jsonl_line_unlocked_with_write(
            &path, line.clone(), AppendDurability::Durable,
            |file, bytes| {
                file.write_all(&bytes[..prefix_len])?;
                Err(io::Error::other("injected partial write"))
            }, |_| Ok(()), || Ok(()),
        );
        assert!(matches!(failed, Err(AppendLineError::Indeterminate(error)) if error.to_string() == "injected partial write"));
    }
    assert_eq!(std::fs::read(&path).unwrap(), line[..prefix_len]);
    let (state, _) = storage.restore_interjections(&info).await.unwrap();
    assert!(matches!(storage.append_interjection_receipt(&info, &state, &original).await.unwrap(), InterjectionAppend::Appended(_)));
    let (cold, index) = storage.restore_interjections(&info).await.unwrap();
    assert_eq!(index.receipts().count(), 1);
    assert_eq!(index.get("one").unwrap().source(), original.source());
    match storage.append_interjection_receipt(&info, &cold, &receipt("one", "original", 2)).await.unwrap() {
        InterjectionAppend::AlreadyPresent(saved) => assert_eq!(saved.source(), original.source()),
        _ => panic!("retry must return the complete original admission"),
    }
    assert!(std::fs::read(path).unwrap().starts_with(&line[..prefix_len]));
}

#[tokio::test]
async fn complete_json_without_newline_is_indeterminate_and_later_append_preserves_its_identity() {
    let (_dir, storage, info) = fixture().await;
    let path = storage.chat_file(&info);
    let original = receipt("one", "original", 1);
    let mut line = serde_json::to_vec(&ReceiptRecord { interjection_receipt: original.clone() }).unwrap();
    line.push(b'\n');
    {
        let _lock = JsonlStorageAdapter::lock_append(&path).unwrap();
        let failed = JsonlStorageAdapter::append_jsonl_line_unlocked_with_write(
            &path, line.clone(), AppendDurability::Durable,
            |file, bytes| {
                file.write_all(&bytes[..bytes.len() - 1])?;
                Err(io::Error::other("injected missing newline"))
            }, |_| Ok(()), || Ok(()),
        );
        assert!(matches!(failed, Err(AppendLineError::Indeterminate(error)) if error.to_string() == "injected missing newline"));
    }
    assert_eq!(std::fs::read(&path).unwrap(), line[..line.len() - 1]);
    let (writer, _) = storage.restore_interjections(&info).await.unwrap();
    storage.append_interjection_receipt(&info, &writer, &receipt("two", "other", 3)).await.unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(&line));
    let (cold, index) = storage.restore_interjections(&info).await.unwrap();
    assert_eq!(index.receipts().count(), 2);
    match storage.append_interjection_receipt(&info, &cold, &receipt("one", "original", 2)).await.unwrap() {
        InterjectionAppend::AlreadyPresent(saved) => {
            assert_eq!(saved.source(), original.source());
            assert_eq!(saved.identity(), original.identity());
            assert_eq!(saved.images(), original.images());
        }
        _ => panic!("a failed write may already contain the original receipt"),
    }
}

#[tokio::test]
async fn repeated_raw_projection_records_materialize_only_one_model_message() {
    let (_dir, storage, info) = fixture().await;
    let original = receipt("one", "original", 1);
    let mut item = ConversationItem::interjection(xai_interjection_core::format_interjection("original".into()));
    if let ConversationItem::User(user) = &mut item {
        user.source = original.source();
        user.interjection = Some(original.identity().clone());
    }
    item.add_image("data:image/png;base64,AA==");
    let mut bytes = serde_json::to_vec(&ReceiptRecord { interjection_receipt: original.clone() }).unwrap();
    bytes.push(b'\n');
    let record = ProjectionRecord { interjection_projection: InterjectionProjection::Projected {
        receipt: original.with_disposition(InterjectionDisposition::Projected).unwrap(), item,
    } };
    let mut projected = serde_json::to_vec(&record).unwrap();
    projected.push(b'\n');
    bytes.extend_from_slice(&projected);
    bytes.extend_from_slice(&projected);
    let path = storage.chat_file(&info);
    std::fs::write(&path, bytes).unwrap();
    let messages = storage.read_chat_history_sync(path, CHAT_FORMAT_VERSION).unwrap();
    assert_eq!(messages.len(), 1);
    let (_, index) = storage.restore_interjections(&info).await.unwrap();
    assert_eq!(index.receipts().count(), 1);
    assert_eq!(index.get("one").unwrap().disposition(), InterjectionDisposition::Projected);
}

#[tokio::test]
async fn contradictory_raw_receipt_transition_fails_cold_restore() {
    let (_dir, storage, info) = fixture().await;
    let original = receipt("one", "original", 1);
    let mut bytes = Vec::new();
    for disposition in [InterjectionDisposition::Pending, InterjectionDisposition::Retired, InterjectionDisposition::Projected] {
        bytes.extend(serde_json::to_vec(&ReceiptRecord {
            interjection_receipt: original.with_disposition(disposition).unwrap(),
        }).unwrap());
        bytes.push(b'\n');
    }
    std::fs::write(storage.chat_file(&info), bytes).unwrap();
    assert!(matches!(storage.restore_interjections(&info).await, Err(InterjectionStorageError::NotCommitted(error)) if error.kind() == io::ErrorKind::Other));
    assert!(storage.read_chat_history_sync(storage.chat_file(&info), CHAT_FORMAT_VERSION).is_err());
}

#[tokio::test]
async fn lookup_refreshes_the_live_cursor_after_the_supported_atomic_rewrite() {
    let (_dir, storage, info) = fixture().await;
    let original = receipt("one", "original", 1);
    let (state, _) = storage.restore_interjections(&info).await.unwrap();
    storage.append_interjection_receipt(&info, &state, &original).await.unwrap();
    storage.persist_interjection_projection(&info, &state, InterjectionProjection::Retired {
        receipt: original.with_disposition(InterjectionDisposition::Retired).unwrap(),
    }).await.unwrap();
    storage.replace_chat_history(&info, &[ConversationItem::user("rewritten".repeat(1000))]).await.unwrap();
    let live = storage.lookup_interjection_receipt(&info, &state, "one").await.unwrap().unwrap();
    let (_, cold) = storage.restore_interjections(&info).await.unwrap();
    let saved = cold.get("one").unwrap();
    assert_eq!(live.source(), saved.source());
    assert_eq!(live.identity(), saved.identity());
    assert_eq!(live.disposition(), InterjectionDisposition::Retired);
    match storage.append_interjection_receipt(&info, &state, &receipt("one", "original", 2)).await.unwrap() {
        InterjectionAppend::AlreadyPresent(saved) => assert_eq!(saved.source(), original.source()),
        _ => panic!("the atomic rewrite must not replace admission authority"),
    }
}
