use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use crate::session::persistence::{Info, PersistenceMsg, durable_update_tests::test_actor};
use crate::session::request_capture::*;
use crate::session::storage::{StorageAdapter, SessionUpdate, SessionUpdateEnvelope, request_capture};
use crate::session::storage::jsonl::JsonlStorageAdapter;
use xai_grok_sampling_types::{SourceOccurrence, SourceOccurrenceId};

fn publication(info: &Info, number: u8) -> SessionUpdate {
    SessionUpdate::Xai(Box::new(crate::extensions::notification::SessionNotification {
        session_id: info.id.clone(),
        update: crate::extensions::notification::SessionUpdate::InjectedContextRecord {
            kind: "memory_reminder".into(), text: format!("published occurrence {number}"),
        },
        meta: Some(serde_json::json!({ "requestSource": SourcePublication {
            source: SourceOccurrence::Admitted { id: SourceOccurrenceId([number; 16]) },
            evidence: Default::default(), durability: SourceDurability::UnconfirmedIndependentHistoryChannel,
        }})),
    }))
}

#[tokio::test]
async fn ordered_partial_prefix_is_synced_before_ledger_and_survives_lost_or_lagged_index() {
    let dir = tempfile::tempdir().unwrap();
    let info = Info { id: agent_client_protocol::SessionId::new("publication-prefix"), cwd: "/test".into() };
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let before_barrier = dir.path().to_owned();
    let storage = Arc::new(JsonlStorageAdapter::with_probes(
        dir.path().to_owned(),
        move |_| {
            if observed.fetch_add(1, Ordering::SeqCst) == 1 {
                Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
            } else { Ok(()) }
        },
        move |files| {
            if files.updates {
                assert!(!before_barrier.join(request_capture::DIRECTORY).join("source-publications.jsonl").exists(), "ledger cannot precede journal barrier");
                let rows = std::fs::read_to_string(before_barrier.join(crate::session::storage::UPDATES_FILE)).unwrap();
                let updates: Vec<_> = rows.lines().map(|line| SessionUpdateEnvelope::from_str(line).unwrap()).collect();
                assert_eq!(updates.len(), 1, "only the ordered applied prefix reached the journal");
            }
            Ok(())
        },
    ));
    storage.init_session(&info, agent_client_protocol::ModelId::new("test")).await.unwrap();
    let actor = test_actor(info.clone(), storage);
    let (respond_to, response) = tokio::sync::oneshot::channel();
    actor.handle.tx.send(PersistenceMsg::AppendUpdatesAndAck {
        updates: vec![publication(&info, 1), publication(&info, 2), publication(&info, 3)], respond_to,
    }).unwrap();
    let failure = response.await.unwrap().unwrap_err();
    assert!(matches!(failure.progress, PublicationProgress::JournalPrefix { committed: 1, durability: PublicationDurability::Synced }));
    let expected = vec![SourceOccurrenceId([1; 16])];
    assert_eq!(request_capture::published_sources(dir.path()).unwrap(), expected);
    let ledger = dir.path().join(request_capture::DIRECTORY).join("source-publications.jsonl");
    let journal = std::fs::read(dir.path().join(crate::session::storage::UPDATES_FILE)).unwrap();
    std::fs::write(&ledger, b"").unwrap();
    assert_eq!(request_capture::published_sources(dir.path()).unwrap(), expected, "lagged ledger is not publication authority");
    std::fs::remove_file(&ledger).unwrap();
    assert_eq!(request_capture::published_sources(dir.path()).unwrap(), expected, "journal reconstructs lost ledger");
    assert_eq!(std::fs::read(dir.path().join(crate::session::storage::UPDATES_FILE)).unwrap(), journal);
    actor.task.abort();
}

#[tokio::test]
async fn failed_journal_barrier_creates_no_claim_and_complete_journal_can_be_reconciled() {
    let dir = tempfile::tempdir().unwrap();
    let info = Info { id: agent_client_protocol::SessionId::new("publication-barrier"), cwd: "/test".into() };
    let storage = Arc::new(JsonlStorageAdapter::with_probes(
        dir.path().to_owned(), |_| Ok(()),
        |_| Err(std::io::Error::from(std::io::ErrorKind::Interrupted)),
    ));
    storage.init_session(&info, agent_client_protocol::ModelId::new("test")).await.unwrap();
    let actor = test_actor(info.clone(), storage);
    let (respond_to, response) = tokio::sync::oneshot::channel();
    actor.handle.tx.send(PersistenceMsg::AppendUpdatesAndAck { updates: vec![publication(&info, 4)], respond_to }).unwrap();
    let failure = response.await.unwrap().unwrap_err();
    assert!(matches!(failure.progress, PublicationProgress::JournalPrefix { committed: 1, durability: PublicationDurability::Unconfirmed }));
    assert!(!dir.path().join(request_capture::DIRECTORY).join("source-publications.jsonl").exists());
    let reopened = JsonlStorageAdapter::with_explicit_session_dir(dir.path().to_owned());
    reopened.sync_session_files_selected(&info, crate::session::storage::SessionFileSet { updates: true, ..Default::default() }).await.unwrap();
    assert_eq!(request_capture::published_sources(dir.path()).unwrap(), vec![SourceOccurrenceId([4; 16])]);
    actor.task.abort();
}

#[tokio::test]
async fn phantom_ledger_claims_do_not_hide_unpublished_sources_and_corrupt_ledger_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(request_capture::DIRECTORY);
    std::fs::create_dir(&root).unwrap();
    let ledger = root.join("source-publications.jsonl");
    let phantom = format!("{}\n", serde_json::to_string(&SourceOccurrenceId([9; 16])).unwrap());
    std::fs::write(&ledger, phantom).unwrap();
    assert!(request_capture::published_sources(dir.path()).unwrap().is_empty());
    let future = b"{\"futureVersion\":17}\n";
    std::fs::write(&ledger, future).unwrap();
    let error = request_capture::published_sources(dir.path()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read(&ledger).unwrap(), future);
}

#[tokio::test]
async fn committed_bookkeeping_failure_keeps_the_written_record_in_the_synced_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let info = Info { id: agent_client_protocol::SessionId::new("publication-bookkeeping"), cwd: "/test".into() };
    let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(dir.path().to_owned()));
    storage.init_session(&info, agent_client_protocol::ModelId::new("test")).await.unwrap();
    let summary = dir.path().join(crate::session::storage::SUMMARY_FILE);
    std::fs::remove_file(&summary).unwrap();
    std::fs::create_dir(&summary).unwrap();
    let actor = test_actor(info.clone(), storage);
    let (respond_to, response) = tokio::sync::oneshot::channel();
    actor.handle.tx.send(PersistenceMsg::AppendUpdatesAndAck {
        updates: vec![publication(&info, 5), publication(&info, 6)], respond_to,
    }).unwrap();
    let failure = response.await.unwrap().unwrap_err();
    assert!(matches!(failure.progress, PublicationProgress::JournalPrefix { committed: 1, durability: PublicationDurability::Synced }));
    let rows = std::fs::read_to_string(dir.path().join(crate::session::storage::UPDATES_FILE)).unwrap();
    assert_eq!(rows.lines().count(), 1, "bookkeeping failure must not roll back the real append or guess that the suffix applied");
    assert_eq!(request_capture::published_sources(dir.path()).unwrap(), vec![SourceOccurrenceId([5; 16])]);
    actor.task.abort();
}

#[tokio::test]
async fn actual_append_after_crashed_fragment_preserves_bytes_and_reconciles_only_complete_typed_records() {
    let dir = tempfile::tempdir().unwrap();
    let info = Info { id: agent_client_protocol::SessionId::new("publication-torn-record"), cwd: "/test".into() };
    let storage = JsonlStorageAdapter::with_explicit_session_dir(dir.path().to_owned());
    storage.init_session(&info, agent_client_protocol::ModelId::new("test")).await.unwrap();
    let updates = dir.path().join(crate::session::storage::UPDATES_FILE);
    let interrupted = b"{\"partial\":";
    std::fs::write(&updates, interrupted).unwrap();
    storage.append_update(&info, &publication(&info, 7)).await.unwrap();
    let before = std::fs::read(&updates).unwrap();
    assert!(before.starts_with(interrupted));
    assert_eq!(request_capture::published_sources(dir.path()).unwrap(), vec![SourceOccurrenceId([7; 16])]);
    assert_eq!(std::fs::read(&updates).unwrap(), before, "reconciliation cannot truncate crashed bytes or rewrite journal history");
    std::fs::write(&updates, b"{\"futureVersion\":17}\n").unwrap();
    assert_eq!(request_capture::published_sources(dir.path()).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read(&updates).unwrap(), b"{\"futureVersion\":17}\n");
}
