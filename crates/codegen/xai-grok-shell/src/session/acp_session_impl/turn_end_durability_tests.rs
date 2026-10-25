use super::*;
use super::super::support::{create_test_actor_with_chat_persistence, input_with_origin_rx, running_task_stub};
use crate::session::storage::{SessionUpdate, StorageAdapter};

#[tokio::test]
async fn caller_completion_waits_for_replayable_terminal_commit() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway_tx, _gateway_rx) = tokio::sync::mpsc::unbounded_channel();
        let (persistence_tx, mut persistence_rx) = tokio::sync::mpsc::unbounded_channel();
        let (actor, _) = create_test_actor_with_chat_persistence(0, 256_000, 85, gateway_tx, persistence_tx, Box::new(xai_chat_state::NullChatPersistence)).await;
        let (input, mut response) = input_with_origin_rx("durable-caller", crate::session::PromptOrigin::User);
        let task = running_task_stub("durable-caller");
        let epoch = task.epoch;
        let identity = task.identity.clone();
        {
            let mut state = actor.state.lock().await;
            state.pending_inputs.push_back(input);
            state.running_task = Some(task);
        }
        let root = tempfile::tempdir().unwrap();
        let storage = crate::session::storage::jsonl::JsonlStorageAdapter::with_explicit_session_dir(root.path().to_path_buf());
        storage.init_session(&actor.session_info, acp::ModelId::new("test")).await.unwrap();
        let complete = actor.handle_completion("durable-caller".into(), epoch, &identity, ok_end_turn(0, None), None);
        let persist = async {
            loop {
                match persistence_rx.recv().await.unwrap() {
                    PersistenceMsg::AppendUpdateDurablyAndAck { update, respond_to } => {
                    assert!(matches!(response.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)), "caller succeeded before terminal persistence");
                    assert!(matches!(&update, SessionUpdate::Xai(notification) if matches!(&notification.update, XaiSessionUpdate::TurnCompleted { prompt_id, .. } if prompt_id == "durable-caller")));
                    let result = storage.append_update_durable_commit_aware(&actor.session_info, &update).await;
                    assert!(result.is_ok());
                    assert!(matches!(response.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
                    respond_to.send(result).unwrap();
                    }
                    PersistenceMsg::FinishDeliveryTurn { turn_id, parent_task, origin, progress, respond_to } => {
                        assert!(matches!(response.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)), "caller succeeded before durable settlement");
                        let result = storage.finish_delivery_turn(&actor.session_info, &turn_id, &parent_task, &origin, &progress).await;
                        assert!(result.is_ok());
                        respond_to.send(result).unwrap();
                        break;
                    }
                    _ => {}
                }
            }
        };
        let (owned, ()) = tokio::join!(complete, persist);
        assert!(owned);
        assert!(response.await.unwrap().is_ok());
        let reopened = crate::session::storage::jsonl::JsonlStorageAdapter::with_explicit_session_dir(root.path().to_path_buf());
        let path = reopened.updates_file_path(&actor.session_info).unwrap();
        let lines = std::fs::read_to_string(path).unwrap();
        assert_eq!(lines.lines().filter(|line| {
            let update = crate::session::storage::SessionUpdateEnvelope::from_str(line).unwrap();
            matches!(update, SessionUpdate::Xai(notification) if matches!(notification.update, XaiSessionUpdate::TurnCompleted { prompt_id, .. } if prompt_id == "durable-caller"))
        }).count(), 1);
    }).await;
}

#[tokio::test]
async fn failed_or_lost_terminal_ack_never_returns_caller_success() {
    tokio::task::LocalSet::new().run_until(async {
        for lose_ack in [false, true] {
            let (gateway_tx, _gateway_rx) = tokio::sync::mpsc::unbounded_channel();
            let (persistence_tx, mut persistence_rx) = tokio::sync::mpsc::unbounded_channel();
            let (actor, _) = create_test_actor_with_chat_persistence(0, 256_000, 85, gateway_tx, persistence_tx, Box::new(xai_chat_state::NullChatPersistence)).await;
            let (input, mut response) = input_with_origin_rx("failed-caller", crate::session::PromptOrigin::User);
            let task = running_task_stub("failed-caller");
            let epoch = task.epoch;
            let identity = task.identity.clone();
            {
                let mut state = actor.state.lock().await;
                state.pending_inputs.push_back(input);
                state.running_task = Some(task);
            }
            let complete = actor.handle_completion("failed-caller".into(), epoch, &identity, ok_end_turn(0, None), None);
            let reject = async {
                loop {
                    if let PersistenceMsg::AppendUpdateDurablyAndAck { respond_to, .. } = persistence_rx.recv().await.unwrap() {
                        assert!(matches!(response.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
                        if lose_ack {
                            drop(respond_to);
                        } else {
                            respond_to.send(Err(crate::session::storage::AppendUpdateError::NotCommitted(
                                std::io::Error::new(std::io::ErrorKind::StorageFull, "terminal storage full"),
                            ))).unwrap();
                        }
                        break;
                    }
                }
            };
            let (owned, ()) = tokio::join!(complete, reject);
            assert!(owned);
            assert!(response.await.unwrap().is_err());
            assert!(actor.state.lock().await.running_task.is_none());
            while let Ok(message) = persistence_rx.try_recv() {
                assert!(!matches!(message, PersistenceMsg::FinishDeliveryTurn { .. }), "failed terminal must retain completion ownership");
            }
        }
    }).await;
}
