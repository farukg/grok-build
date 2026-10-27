use super::*;
use xai_grok_sampler::{ApiBackend, RequestId, RetryPolicy, SamplerActor, SamplerConfig, SamplingEvent};
use xai_grok_sampling_types::{CompactionCoverage, ContentTransform, ConversationItem, ConversationRequest, CoverageKnowledge, SourceOccurrence, SourceOccurrenceId};

pub(crate) async fn captured(text: &str) -> (RequestSnapshotId, Vec<(RequestAttemptId, LocalRequestEvidence)>, Vec<u8>) {
    let server = xai_grok_test_support::MockInferenceServer::start().await.unwrap();
    server.set_response("capture fixture completed");
    server.set_keep_requests(true);
    server.set_capture_request_bytes(true);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = SamplerActor::spawn(SamplerConfig {
        base_url: server.url(), model: "test".into(), api_backend: ApiBackend::Messages,
        max_retries: Some(0), request_compression: xai_grok_sampler::RequestCompression::None,
        ..Default::default()
    }, RetryPolicy::default(), tx);
    let mut item = ConversationItem::user(text);
    *item.source_mut() = SourceOccurrence::Admitted { id: SourceOccurrenceId([31; 16]) };
    item.evidence_mut().record(ContentTransform::ImagesEvicted { kept: 1, total: 2 });
    item.evidence_mut().set_compaction(CompactionCoverage::Replaced {
        sources: vec![SourceOccurrenceId([32; 16])], inherited: vec![SourceOccurrenceId([33; 16])],
        unidentified: 1, ancestry: CoverageKnowledge::Unknown,
    });
    let id = RequestSnapshotId(uuid::Uuid::new_v4());
    handle.submit_and_collect(RequestId::from(id.0.to_string()), ConversationRequest {
        items: vec![item], ..Default::default()
    }).await.unwrap();
    let events = std::iter::from_fn(|| rx.try_recv().ok()).filter_map(|event| {
        if let SamplingEvent::LocalRequest { attempt, evidence, .. } = event { Some((attempt, evidence)) }
        else { None }
    }).collect::<Vec<_>>();
    let wire = server.requests().into_iter().find_map(|request| request.raw_body).unwrap().into_bytes();
    assert!(events.iter().any(|(_, evidence)| matches!(evidence, LocalRequestEvidence::Prepared { .. })));
    (id, events, wire)
}

fn origin(request: RequestSnapshotId) -> RequestOrigin {
    RequestOrigin {
        session_id: "stored-session".into(), request, turn: 7, generation: 9,
        prompt_id: Some("prompt-origin".into()), conversation_id: Some("conversation-origin".into()), logical_request_id: Some("logical-origin".into()),
    }
}

fn snapshot(dir: &Path, id: RequestSnapshotId) -> RequestSnapshot {
    let SnapshotReadResult::Snapshot { snapshot } = read(dir, "stored-session", SnapshotRead::Get { request: id }).unwrap() else { panic!("stored snapshot"); };
    snapshot
}

#[tokio::test]
async fn exact_private_capture_and_public_redaction_survive_reopen_without_history_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let (id, events, wire) = captured("/home/capture-owner/private-project/body-input").await;
    let original = b"human and harness history must remain byte-identical\n";
    std::fs::write(directory.path().join(super::super::CHAT_HISTORY_FILE), original).unwrap();
    begin(directory.path(), &origin(id)).unwrap();
    let mut body_digest = None;
    for (attempt, evidence) in events {
        if let LocalRequestEvidence::Prepared { body, .. } = &evidence {
            assert_eq!(body.sensitive_local_bytes(), wire);
            assert!(!format!("{evidence:?}").contains("body-input"));
            body_digest = Some(*body.digest().as_bytes());
        }
        let notification = record(directory.path(), "stored-session", id, attempt, evidence);
        assert!(matches!(notification.receipt, CaptureReceipt::Confirmed { .. }));
        assert!(!serde_json::to_string(&notification).unwrap().contains("body-input"));
    }
    assert_eq!(std::fs::read(directory.path().join(super::super::CHAT_HISTORY_FILE)).unwrap(), original);
    let digest = body_digest.unwrap();
    let root = directory.path().join(DIRECTORY);
    assert_eq!(std::fs::read(blob_path(&root, digest, "raw")).unwrap(), wire);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&root).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(blob_path(&root, digest, "raw")).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let retained = snapshot(directory.path(), id);
    assert_eq!(retained.origin, origin(id));
    assert!(retained.observations.iter().any(|entry| matches!(&entry.observation, RequestObservation::Finished { outcome: AttemptOutcome::Completed })));
    let SnapshotReadResult::Body { body } = read(directory.path(), "stored-session", SnapshotRead::Body { request: id, attempt: RequestAttemptId(1), selection: BodySelection::Digest { digest } }).unwrap() else { panic!("body read"); };
    assert!(!format!("{body:?}").contains("body-input"));
    let RequestBodyRead::Available { json, representation, protected_local } = body else { panic!("public body"); };
    assert!(matches!(representation, xai_grok_sampler::request_capture::PayloadRepresentation::Redacted { original_digest } if original_digest == digest));
    assert!(!json.contains("/home/capture-owner"));
    assert_eq!(protected_local.digest, digest);
    std::fs::write(blob_path(&root, digest, "view"), b"{\"altered\":true}").unwrap();
    assert!(read(directory.path(), "stored-session", SnapshotRead::Body {
        request: id, attempt: RequestAttemptId(1), selection: BodySelection::Digest { digest },
    }).is_err(), "a public payload must match the publication digest recorded for this actual Prepared body");
    let RequestObservation::Prepared { membership, source_durability, .. } = &retained.observations[0].observation else { panic!("prepared first"); };
    assert!(matches!(source_durability, SourceDurability::UnconfirmedIndependentHistoryChannel));
    assert_eq!(membership[0].source, SourceOccurrence::Admitted { id: SourceOccurrenceId([31; 16]) });
    assert_eq!(membership[0].evidence.transforms(), &[ContentTransform::ImagesEvicted { kept: 1, total: 2 }]);
    assert!(matches!(membership[0].evidence.compaction(), CompactionCoverage::Replaced { sources, inherited, unidentified: 1, ancestry: CoverageKnowledge::Unknown }
        if sources == &[SourceOccurrenceId([32; 16])] && inherited == &[SourceOccurrenceId([33; 16])]));
}

#[tokio::test]
async fn attempts_share_one_blob_and_partial_cancelled_http_failures_are_independent_facts() {
    let directory = tempfile::tempdir().unwrap();
    let (id, events, _) = captured("shared retry body").await;
    let prepared = events.into_iter().find_map(|(_, evidence)| matches!(evidence, LocalRequestEvidence::Prepared { .. }).then_some(evidence)).unwrap();
    begin(directory.path(), &origin(id)).unwrap();
    let mut expected_events = Vec::new();
    for (attempt, observations) in [
        (1, vec![prepared.clone(), LocalRequestEvidence::Finished { outcome: xai_grok_sampler::request_capture::LocalAttemptOutcome::InitializationFailed }]),
        (2, vec![prepared.clone(), LocalRequestEvidence::DispatchStarted, LocalRequestEvidence::ResponseHeaders { status: 503 }, LocalRequestEvidence::Finished { outcome: xai_grok_sampler::request_capture::LocalAttemptOutcome::Failed }]),
        (3, vec![prepared.clone(), LocalRequestEvidence::DispatchStarted, LocalRequestEvidence::Finished { outcome: xai_grok_sampler::request_capture::LocalAttemptOutcome::Cancelled }]),
        (4, vec![prepared]),
    ] {
        for evidence in observations {
            expected_events.push((attempt, match &evidence {
                LocalRequestEvidence::Prepared { .. } => "prepared", LocalRequestEvidence::DispatchStarted => "dispatch",
                LocalRequestEvidence::ResponseHeaders { .. } => "headers", LocalRequestEvidence::Finished { .. } => "finished",
            }));
            assert!(matches!(record(directory.path(), "stored-session", id, RequestAttemptId(attempt), evidence).receipt, CaptureReceipt::Confirmed { .. }));
        }
    }
    let retained = snapshot(directory.path(), id);
    let actual = retained.observations.iter().map(|entry| (entry.attempt.0, match &entry.observation {
        RequestObservation::Prepared { .. } => "prepared", RequestObservation::DispatchStarted => "dispatch",
        RequestObservation::ResponseHeaders { .. } => "headers", RequestObservation::Finished { .. } => "finished",
    })).collect::<Vec<_>>();
    assert_eq!(actual, expected_events);
    assert_eq!(load_cache(&directory.path().join(DIRECTORY)).unwrap().entries.len(), 1);
    assert!(matches!(&retained.observations.last().unwrap().observation, RequestObservation::Prepared { .. }), "partial is not a fabricated Finished or consumed claim");
}

#[tokio::test]
async fn recent_raw_eviction_keeps_all_request_source_outcome_references_and_user_files() {
    let directory = tempfile::tempdir().unwrap();
    let mut ids = Vec::new();
    let mut first_digest = None;
    for index in 0..=MAX_CACHE_BLOBS {
        let (id, events, _) = captured(&format!("bounded-cache-input-{index}")).await;
        begin(directory.path(), &origin(id)).unwrap();
        for (attempt, evidence) in events {
            if index == 0 {
                if let LocalRequestEvidence::Prepared { body, .. } = &evidence { first_digest = Some(*body.digest().as_bytes()); }
            }
            record(directory.path(), "stored-session", id, attempt, evidence);
        }
        ids.push(id);
        std::fs::write(directory.path().join(DIRECTORY).join("user-work.txt"), b"never evict user work").unwrap();
    }
    let root = directory.path().join(DIRECTORY);
    let cache = load_cache(&root).unwrap();
    assert_eq!(cache.entries.len(), MAX_CACHE_BLOBS);
    assert!(cache.entries.iter().map(|entry| entry.bytes).sum::<u64>() <= MAX_CACHE_BYTES);
    let first = snapshot(directory.path(), ids[0]);
    assert!(matches!(&first.observations[0].observation, RequestObservation::Prepared { body: PreparedBody { digest, availability: BodyAvailability::Expired, .. }, membership, .. }
        if Some(*digest) == first_digest && membership.len() == 1));
    assert!(first.observations.iter().any(|entry| matches!(entry.observation, RequestObservation::Finished { outcome: AttemptOutcome::Completed })));
    let SnapshotReadResult::List { page } = read(directory.path(), "stored-session", SnapshotRead::List { after: 0, limit: 100 }).unwrap() else { panic!("index"); };
    assert_eq!(page.requests.iter().map(|entry| entry.request).collect::<Vec<_>>(), ids);
    assert_eq!(std::fs::read(root.join("user-work.txt")).unwrap(), b"never evict user work");
    let last = cache.entries.last().unwrap();
    std::fs::remove_file(blob_path(&root, last.digest, "raw")).unwrap();
    let last = snapshot(directory.path(), *ids.last().unwrap());
    assert!(matches!(&last.observations[0].observation, RequestObservation::Prepared { body: PreparedBody { availability: BodyAvailability::Unknown, .. }, .. }));
}

#[tokio::test]
async fn oversize_storage_failure_and_lookup_tampering_never_fabricate_capture_success() {
    let directory = tempfile::tempdir().unwrap();
    let (id, events, wire) = captured("independent lookup input").await;
    let prepared = events.into_iter().find_map(|(_, evidence)| matches!(evidence, LocalRequestEvidence::Prepared { .. }).then_some(evidence)).unwrap();
    begin(directory.path(), &origin(id)).unwrap();
    let _scope = super::super::AtomicWriteFaultScope::enter();
    super::super::fail_next_atomic_write(super::super::AtomicWriteFault::Write);
    let notification = record(directory.path(), "stored-session", id, RequestAttemptId(1), prepared);
    assert!(matches!(notification.receipt, CaptureReceipt::Confirmed { .. }), "failed raw capture is itself durable lightweight evidence");
    assert!(matches!(&notification.evidence.observation, RequestObservation::Prepared { body: PreparedBody {
        digest, byte_len, availability: BodyAvailability::Failed { .. },
    }, .. } if digest == blake3::hash(&wire).as_bytes() && *byte_len == wire.len() as u64));
    super::super::fail_next_atomic_write(super::super::AtomicWriteFault::Write);
    let failed = record(directory.path(), "stored-session", id, RequestAttemptId(1), LocalRequestEvidence::DispatchStarted);
    assert!(matches!(failed.receipt, CaptureReceipt::Unconfirmed { .. }));
    assert!(read(directory.path(), "another-session", SnapshotRead::Get { request: id }).is_err());
    assert!(serde_json::from_value::<SnapshotRead>(serde_json::json!({"action":"get","request":"../../outside"})).is_err());
    let mut changed = snapshot(directory.path(), id);
    changed.origin.request = RequestSnapshotId(uuid::Uuid::new_v4());
    save_json(&snapshot_path(&directory.path().join(DIRECTORY), id), &changed).unwrap();
    assert!(read(directory.path(), "stored-session", SnapshotRead::Get { request: id }).is_err());
}

#[tokio::test]
async fn oversized_exact_bodies_are_not_truncated_and_metadata_remains_available() {
    let directory = tempfile::tempdir().unwrap();
    let input = "z".repeat(MAX_BODY_BYTES as usize);
    let (id, events, wire) = captured(&input).await;
    assert!(wire.len() as u64 > MAX_BODY_BYTES);
    begin(directory.path(), &origin(id)).unwrap();
    for (attempt, evidence) in events { record(directory.path(), "stored-session", id, attempt, evidence); }
    let snapshot = snapshot(directory.path(), id);
    assert!(matches!(&snapshot.observations[0].observation, RequestObservation::Prepared {
        body: PreparedBody { digest, byte_len, availability: BodyAvailability::NotCaptured { reason: NotCapturedReason::Oversize } }, membership, ..
    } if membership.len() == 1 && digest == blake3::hash(&wire).as_bytes() && *byte_len == wire.len() as u64));
    assert!(snapshot.observations.iter().any(|entry| matches!(entry.observation, RequestObservation::Finished { outcome: AttemptOutcome::Completed })));
    assert!(load_cache(&directory.path().join(DIRECTORY)).unwrap().entries.is_empty());
    let SnapshotReadResult::Body { body: RequestBodyRead::Unavailable { availability } } = read(directory.path(), "stored-session", SnapshotRead::Body {
        request: id, attempt: RequestAttemptId(1), selection: BodySelection::LatestPrepared,
    }).unwrap() else { panic!("oversize body unavailable"); };
    assert!(matches!(availability, BodyAvailability::NotCaptured { reason: NotCapturedReason::Oversize }));
}

#[cfg(unix)]
#[test]
fn capture_owner_rejects_dangling_index_lock_links_without_touching_outside_work() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = prepare_root(directory.path()).unwrap();
    let target = outside.path().join("user-work.txt");
    std::os::unix::fs::symlink(&target, root.join("capture.lock")).unwrap();
    assert!(begin(directory.path(), &origin(RequestSnapshotId(uuid::Uuid::new_v4()))).is_err());
    assert!(!target.exists());
    assert!(!root.join("requests.jsonl").exists());
}

#[tokio::test]
async fn abandoned_writing_intent_recaptures_same_digest_without_evicting_other_work() {
    let directory = tempfile::tempdir().unwrap();
    let (id, events, wire) = captured("writing recovery input").await;
    let prepared = events.into_iter().find_map(|(_, evidence)| matches!(evidence, LocalRequestEvidence::Prepared { .. }).then_some(evidence)).unwrap();
    let LocalRequestEvidence::Prepared { body, .. } = &prepared else { panic!("producer prepared body"); };
    let digest = *body.digest().as_bytes();
    let view = body.publication_view().unwrap();
    begin(directory.path(), &origin(id)).unwrap();
    let root = directory.path().join(DIRECTORY);
    let foreign = root.join("user-work.txt");
    std::fs::write(&foreign, b"retain unrelated work").unwrap();
    for window in 0..3 {
        let cache = BodyCache { entries: vec![CacheEntry {
            digest, bytes: wire.len() as u64 + view.bytes.len() as u64,
            byte_len: wire.len() as u64, publication_digest: *blake3::hash(&view.bytes).as_bytes(),
            representation: view.representation, state: CacheState::Writing,
        }] };
        save_json(&root.join("bodies.json"), &cache).unwrap();
        remove_blob(&root, digest, "raw").unwrap();
        remove_blob(&root, digest, "view").unwrap();
        if window >= 1 { save_text(&blob_path(&root, digest, "raw"), std::str::from_utf8(&wire).unwrap()).unwrap(); }
        if window >= 2 { save_text(&blob_path(&root, digest, "view"), std::str::from_utf8(&view.bytes).unwrap()).unwrap(); }
        {
            let owner = lock(&root).unwrap();
            let blocked = record(directory.path(), "stored-session", id, RequestAttemptId(10 + window), prepared.clone());
            assert!(matches!(blocked.receipt, CaptureReceipt::Unconfirmed { .. }), "a live writer cannot be displaced");
            drop(owner);
        }
        let receipt = record(directory.path(), "stored-session", id, RequestAttemptId(10 + window), prepared.clone());
        assert!(matches!(receipt.receipt, CaptureReceipt::Confirmed { .. }));
        assert!(matches!(receipt.evidence.observation, RequestObservation::Prepared { body: PreparedBody { availability: BodyAvailability::Captured { .. }, .. }, .. }));
        assert_eq!(std::fs::read(blob_path(&root, digest, "raw")).unwrap(), wire);
        let cache = load_cache(&root).unwrap();
        assert_eq!(cache.entries.len(), 1);
        assert!(matches!(cache.entries[0].state, CacheState::Ready));
        assert_eq!(cache.entries[0].bytes, wire.len() as u64 + view.bytes.len() as u64);
        assert_eq!(std::fs::read(&foreign).unwrap(), b"retain unrelated work");
        let SnapshotReadResult::Body { body: RequestBodyRead::Available { json, .. } } = read(directory.path(), "stored-session", SnapshotRead::Body {
            request: id, attempt: RequestAttemptId(10 + window), selection: BodySelection::Digest { digest },
        }).unwrap() else { panic!("recovered publication body"); };
        assert_eq!(json.as_bytes(), view.bytes.as_ref());
    }
}

#[test]
fn capture_capabilities_report_unobserved_purposes_without_creating_a_store() {
    let directory = tempfile::tempdir().unwrap();
    let SnapshotReadResult::Capabilities { scope } = read(directory.path(), "no-session", SnapshotRead::Capabilities).unwrap() else { panic!("typed capture capability"); };
    let value = serde_json::to_value(scope).unwrap();
    assert_eq!(value, serde_json::json!({
        "coverage": "main_turn_only",
        "unobserved": ["compaction", "title", "recap", "summary", "memory_dream_v1", "memory_dream_v2",
            "memory_dream_batch", "goal", "laziness", "image_describe", "permission_classifier"]
    }));
    assert!(!directory.path().join(DIRECTORY).exists());
}

#[test]
fn legacy_publication_failure_without_stage_is_unknown_not_non_submission() {
    let failure: SourcePublicationFailure = serde_json::from_value(serde_json::json!({
        "sessionId": "legacy-session", "sources": [], "cause": "reply_lost"
    })).unwrap();
    assert!(matches!(failure.progress, PublicationProgress::Unknown));
}

fn index_entries(dir: &Path, count: usize) -> RequestOrigin {
    use std::io::Write as _;
    let root = prepare_root(dir).unwrap();
    let last = RequestSnapshotId(uuid::Uuid::new_v4());
    let mut bytes = Vec::new();
    for entry in (1..count).map(|_| RequestSnapshotId(uuid::Uuid::new_v4())).chain([last]) {
        let mut line = serde_json::to_vec(&entry).unwrap();
        line.push(b'\n');
        bytes.extend_from_slice(&line);
    }
    private_append(&root.join("requests.jsonl")).unwrap().write_all(&bytes).unwrap();
    let expected = origin(last);
    save_json(&snapshot_path(&root, last), &RequestSnapshot { origin: expected.clone(), observations: Vec::new() }).unwrap();
    expected
}

#[test]
fn latest_reads_the_index_tail_at_page_and_history_boundaries() {
    for count in [1, 100, 200, 6_401] {
        let directory = tempfile::tempdir().unwrap();
        let expected = index_entries(directory.path(), count);
        let query = |action| read(directory.path(), "stored-session", action).unwrap();
        let SnapshotReadResult::Snapshot { snapshot } = query(SnapshotRead::Latest) else { panic!("latest snapshot at {count} entries"); };
        assert_eq!(snapshot.origin, expected, "latest must resolve the final index entry, not an empty full page");
        let SnapshotReadResult::Snapshot { snapshot: get } = query(SnapshotRead::Get { request: expected.request }) else { panic!("get snapshot"); };
        assert_eq!((snapshot.origin, snapshot.observations.len()), (get.origin, get.observations.len()));
    }
}

#[test]
fn latest_reports_absent_for_missing_or_empty_index() {
    let absent = tempfile::tempdir().unwrap();
    assert!(matches!(read(absent.path(), "no-session", SnapshotRead::Latest).unwrap(), SnapshotReadResult::Absent));
    let directory = tempfile::tempdir().unwrap();
    prepare_root(directory.path()).unwrap();
    std::fs::write(directory.path().join(DIRECTORY).join("requests.jsonl"), b"").unwrap();
    assert!(matches!(read(directory.path(), "stored-session", SnapshotRead::Latest).unwrap(), SnapshotReadResult::Absent));
}

#[test]
fn latest_keeps_malformed_index_tail_errors_instead_of_serving_older_entries() {
    let directory = tempfile::tempdir().unwrap();
    let root = prepare_root(directory.path()).unwrap();
    let index = root.join("requests.jsonl");
    let id = RequestSnapshotId(uuid::Uuid::new_v4());
    let mut valid = serde_json::to_vec(&id).unwrap();
    valid.push(b'\n');
    let mut terminated = valid.clone();
    terminated.extend_from_slice(b"\"not-a-uuid\"\n");
    std::fs::write(&index, &terminated).unwrap();
    assert!(read(directory.path(), "stored-session", SnapshotRead::Latest).is_err(), "a malformed terminated tail must not fall back to the older entry");
    let mut unterminated = valid;
    unterminated.extend_from_slice(b"\"not-a-u");
    std::fs::write(&index, &unterminated).unwrap();
    assert!(read(directory.path(), "stored-session", SnapshotRead::Latest).is_err(), "a malformed unterminated tail keeps its error");
    let last = RequestSnapshotId(uuid::Uuid::new_v4());
    let mut complete = valid;
    complete.extend_from_slice(&serde_json::to_vec(&last).unwrap());
    std::fs::write(&index, &complete).unwrap();
    save_json(&snapshot_path(&root, last), &RequestSnapshot { origin: origin(last), observations: Vec::new() }).unwrap();
    let SnapshotReadResult::Snapshot { snapshot } = read(directory.path(), "stored-session", SnapshotRead::Latest).unwrap() else { panic!("final line without its terminating newline still resolves"); };
    assert_eq!(snapshot.origin.request, last);
}

#[cfg(unix)]
#[test]
fn latest_rejects_non_regular_index_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = prepare_root(directory.path()).unwrap();
    std::os::unix::fs::symlink(std::env::temp_dir().join("user-work.jsonl"), root.join("requests.jsonl")).unwrap();
    assert!(read(directory.path(), "stored-session", SnapshotRead::Latest).is_err());
}
