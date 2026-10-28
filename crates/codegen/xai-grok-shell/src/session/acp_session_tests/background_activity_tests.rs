use super::super::support::create_test_actor;
use crate::extensions::notification::{SessionNotification, SessionUpdate as XaiUpdate};
use crate::session::acp_types::SessionActivityProvenance;
use crate::session::persistence::{PersistenceMsg, Summary};
use crate::session::storage::jsonl::JsonlStorageAdapter;
use crate::session::storage::{SessionUpdate, StorageAdapter};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

#[tokio::test(flavor = "current_thread")]
async fn background_snapshot_cause_survives_persistence_and_wire_without_read_activity() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway_tx, mut gateway_rx) = mpsc::unbounded_channel();
        let (persistence_tx, mut persistence_rx) = mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
        let dir = tempfile::tempdir().unwrap();
        let adapter = JsonlStorageAdapter::with_explicit_session_dir(dir.path().to_path_buf());
        adapter.init_session(&actor.session_info, agent_client_protocol::ModelId::new("test-model"))
            .await.unwrap();
        let summary_path = dir.path().join("summary.json");
        let read_summary = || serde_json::from_slice::<Summary>(
            &std::fs::read(&summary_path).unwrap(),
        ).unwrap();

        for active in [None, chrono::DateTime::from_timestamp_millis(1_234)] {
            let mut baseline = read_summary();
            baseline.last_active_at = active;
            baseline.updated_at = chrono::DateTime::from_timestamp_millis(9_876).unwrap();
            std::fs::write(&summary_path, serde_json::to_vec(&baseline).unwrap()).unwrap();
            let mut expected_activity = baseline.last_active_at;
            let mut expected_updated = baseline.updated_at;
            let mut expected_count = baseline.num_messages;
            for (cause, wire_cause) in [
                (SessionActivityProvenance::Observation, "observation"),
                (SessionActivityProvenance::Observation, "observation"),
                (SessionActivityProvenance::Live, "live"),
                (SessionActivityProvenance::Observation, "observation"),
            ] {
                let pending = Arc::new(AtomicBool::new(true));
                actor.emit_background_tasks_snapshot(cause, Some(pending.clone())).await;
                assert!(!pending.load(Ordering::Acquire));
                let PersistenceMsg::Update(update @ SessionUpdate::Xai(_)) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), persistence_rx.recv())
                        .await.expect("producer persistence must settle")
                        .expect("producer must persist the snapshot")
                else {
                    panic!("expected persisted xAI snapshot");
                };
                let SessionUpdate::Xai(notification) = &update else { unreachable!() };
                assert!(matches!(&notification.update,
                    XaiUpdate::BackgroundTasks { tasks, truncated: false } if tasks.is_empty()));
                assert_eq!(notification.meta.as_ref().unwrap()
                    .get("activityProvenance").and_then(serde_json::Value::as_str), Some(wire_cause));
                let xai_acp_lib::AcpClientMessage::ExtNotification(outbound) =
                    gateway_rx.try_recv().expect("producer must broadcast the snapshot")
                else {
                    panic!("expected xAI extension notification");
                };
                assert_eq!(outbound.request.method.as_ref(), "x.ai/session_notification");
                let wire: SessionNotification = serde_json::from_str(outbound.request.params.get()).unwrap();
                assert_eq!(wire.session_id, actor.session_info.id);
                assert_eq!(wire.meta, notification.meta, "persistence and broadcast share provenance and event identity");
                assert_eq!(serde_json::to_value(wire.update).unwrap(), serde_json::to_value(&notification.update).unwrap());
                adapter.append_update(&actor.session_info, &update).await.unwrap();
                expected_count += 1;
                let after = read_summary();
                assert_eq!(after.num_messages, expected_count, "observations remain persisted");
                match cause {
                    SessionActivityProvenance::Observation => {
                        assert_eq!(after.last_active_at, expected_activity);
                        assert_eq!(after.updated_at, expected_updated);
                    }
                    SessionActivityProvenance::Live => {
                        assert!(after.last_active_at.unwrap().timestamp_millis() > 9_876);
                        expected_activity = after.last_active_at;
                        expected_updated = after.updated_at;
                    }
                }
                assert!(persistence_rx.try_recv().is_err());
                assert!(gateway_rx.try_recv().is_err());
            }
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn gateway_owned_background_state_skips_both_causes_and_releases_coalescing() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway_tx, mut gateway_rx) = mpsc::unbounded_channel();
        let (persistence_tx, mut persistence_rx) = mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
        actor.emit_local_background_tasks.store(false, Ordering::Release);
        for cause in [SessionActivityProvenance::Observation, SessionActivityProvenance::Live] {
            let pending = Arc::new(AtomicBool::new(true));
            actor.emit_background_tasks_snapshot(cause, Some(pending.clone())).await;
            assert!(!pending.load(Ordering::Acquire));
            assert!(persistence_rx.try_recv().is_err());
            assert!(gateway_rx.try_recv().is_err());
        }
    }).await;
}
