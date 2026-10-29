use super::super::support::{
    create_test_actor_with_chat_persistence, input_with_origin_rx, running_task_stub,
};
use super::*;
use crate::session::storage::{SessionUpdate, StorageAdapter};

use super::super::support::user_item_with_rx;

#[derive(Clone, Copy)]
enum NoTaskTerminalCommit {
    Committed,
    WriteFailed,
    AckLost,
}

#[tokio::test(flavor = "current_thread")]
async fn pinned_no_task_cancel_waits_for_real_commit_and_rejects_late_completion() {
    pinned_no_task_cancel_durability(NoTaskTerminalCommit::Committed).await;
}

#[tokio::test(flavor = "current_thread")]
async fn pinned_no_task_cancel_returns_original_write_error_and_preserves_next_rpc() {
    pinned_no_task_cancel_durability(NoTaskTerminalCommit::WriteFailed).await;
}

#[tokio::test(flavor = "current_thread")]
async fn pinned_no_task_cancel_returns_lost_ack_error_and_preserves_next_rpc() {
    pinned_no_task_cancel_durability(NoTaskTerminalCommit::AckLost).await;
}

async fn pinned_no_task_cancel_durability(commit: NoTaskTerminalCommit) {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::oneshot::error::TryRecvError;

    tokio::task::LocalSet::new()
        .run_until(async {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                let root = tempfile::tempdir().unwrap();
                let fail_write = Arc::new(AtomicBool::new(false));
                let probe = fail_write.clone();
                let storage = Arc::new(
                    crate::session::storage::jsonl::JsonlStorageAdapter::with_update_append_probe(
                        root.path().to_path_buf(),
                        move |_| {
                            if probe.load(Ordering::SeqCst) {
                                Err(std::io::Error::new(
                                    std::io::ErrorKind::StorageFull,
                                    "pinned no-task terminal write failed",
                                ))
                            } else {
                                Ok(())
                            }
                        },
                    ),
                );
                let (gateway_tx, _gateway_rx) = tokio::sync::mpsc::unbounded_channel();
                let (proxy_tx, mut requests) = tokio::sync::mpsc::unbounded_channel();
                let (mut actor, _) = create_test_actor_with_chat_persistence(
                    0, 256_000, 85, gateway_tx, proxy_tx,
                    Box::new(xai_chat_state::NullChatPersistence),
                ).await;
                let (subagent_tx, mut subagent_events) = tokio::sync::mpsc::unbounded_channel();
                actor.tool_context.subagent_event_tx = Some(subagent_tx);
                storage.init_session(&actor.session_info, acp::ModelId::new("test"))
                    .await.unwrap();
                let persistence = crate::session::persistence::durable_update_tests::test_actor(
                    actor.session_info.clone(), storage.clone(),
                );
                let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
                let real = persistence.handle.tx.clone();
                let forward = real.clone();
                let (held_tx, held_rx) = tokio::sync::oneshot::channel();
                let terminal_attempts = Arc::new(AtomicUsize::new(0));
                let attempts = terminal_attempts.clone();
                let settlements = Arc::new(AtomicUsize::new(0));
                let forwarded_settlements = settlements.clone();
                // Hold the original request; all replies come from the real persistence actor.
                let _proxy = tokio_util::task::AbortOnDropHandle::new(
                    tokio::task::spawn_local(async move {
                        let mut hold = Some(held_tx);
                        while let Some(request) = requests.recv().await {
                            if let PersistenceMsg::AppendUpdateDurablyAndAck { update, .. } = &request
                                && matches!(update, SessionUpdate::Xai(notification)
                                    if matches!(&notification.update, XaiSessionUpdate::TurnCompleted { .. }))
                            {
                                attempts.fetch_add(1, Ordering::SeqCst);
                                if let Some(hold) = hold.take() {
                                    hold.send(request).unwrap();
                                    continue;
                                }
                            }
                            if matches!(&request, PersistenceMsg::FinishDeliveryTurn { .. }) {
                                forwarded_settlements.fetch_add(1, Ordering::SeqCst);
                            }
                            forward.send(request).unwrap();
                        }
                    }),
                );
                let (front, mut response) = user_item_with_rx("pinned-front", "front-client");
                let (next, mut next_response) = user_item_with_rx("queued-next", "next-client");
                let next_meta = next.queue_meta.clone();
                let next_blocks = serde_json::to_value(&next.prompt_blocks).unwrap();
                {
                    let mut state = actor.state.lock().await;
                    assert!(state.running_task.is_none());
                    state.pending_inputs.push_back(front);
                    state.pending_inputs.push_back(next);
                }
                *actor.current_prompt_id.lock().unwrap() = Some("pinned-front".into());
                let old_task = running_task_stub("pinned-front");
                let report = actor.turn_report.claim_for_gate().expect("report claim");
                let goal_was_active = actor.tool_context.goal_loop_active_gate.load(Ordering::Relaxed);
                let cancel = actor.cancel_running_task(crate::session::CancelOptions {
                    trigger: Some(crate::session::CancelTrigger::CtrlC),
                    user_initiated: true,
                    ..Default::default()
                });
                let commit_terminal = async {
                    let PersistenceMsg::AppendUpdateDurablyAndAck { update, respond_to } =
                        held_rx.await.unwrap()
                    else { panic!("original durable terminal request"); };
                    assert!(matches!(response.try_recv(), Err(TryRecvError::Empty)));
                    assert!(matches!(next_response.try_recv(), Err(TryRecvError::Empty)));
                    assert!(matches!(&update, SessionUpdate::Xai(notification)
                        if matches!(&notification.update, XaiSessionUpdate::TurnCompleted {
                            prompt_id, stop_reason, usage: None, elapsed_ms: None, ..
                        } if prompt_id == "pinned-front" && stop_reason == "cancelled")));
                    fail_write.store(matches!(commit, NoTaskTerminalCommit::WriteFailed), Ordering::SeqCst);
                    let (committed, reply) = tokio::sync::oneshot::channel();
                    real.send(PersistenceMsg::AppendUpdateDurablyAndAck {
                        update, respond_to: committed,
                    }).unwrap();
                    let result = reply.await.unwrap();
                    fail_write.store(false, Ordering::SeqCst);
                    assert!(matches!(response.try_recv(), Err(TryRecvError::Empty)),
                        "even a finished write is not an observed commit before its ACK");
                    match commit {
                        NoTaskTerminalCommit::Committed => {
                            assert!(result.is_ok());
                            respond_to.send(result).unwrap();
                        }
                        NoTaskTerminalCommit::WriteFailed => {
                            let Err(crate::session::storage::AppendUpdateError::NotCommitted(error)) = &result
                            else { panic!("real append must fail before commit"); };
                            assert_eq!(error.kind(), std::io::ErrorKind::StorageFull);
                            assert_eq!(error.to_string(), "pinned no-task terminal write failed");
                            respond_to.send(result).unwrap();
                        }
                        NoTaskTerminalCommit::AckLost => {
                            assert!(result.is_ok(), "lose only the ACK, not the durable write");
                            drop(respond_to);
                        }
                    }
                };
                let (outcome, ()) = tokio::join!(cancel, commit_terminal);
                assert_eq!(outcome.settled, matches!(commit, NoTaskTerminalCommit::Committed));
                assert!(!outcome.turn_stopped);
                let result = response.try_recv().expect("original front receives its result");
                match commit {
                    NoTaskTerminalCommit::Committed => {
                        let result = result.unwrap();
                        assert_eq!(result.stop_reason, acp::StopReason::Cancelled);
                        assert!(matches!(result.completion_kind,
                            crate::session::commands::PromptCompletionKind::Cancelled {
                                category: None, context: None,
                            }));
                        assert!(result.usage.is_none());
                        assert!(result.turn_snapshot.is_none());
                        assert!(result.tool_overrides.is_none());
                    }
                    NoTaskTerminalCommit::WriteFailed => {
                        let expected = crate::session::persistence::io_error_to_acp(
                            &std::io::Error::new(std::io::ErrorKind::StorageFull,
                                "pinned no-task terminal write failed"),
                        );
                        assert_eq!(serde_json::to_value(result.unwrap_err()).unwrap(),
                            serde_json::to_value(expected).unwrap());
                    }
                    NoTaskTerminalCommit::AckLost => {
                        let expected = acp::Error::internal_error()
                            .data("terminal persistence acknowledgement lost");
                        assert_eq!(serde_json::to_value(result.unwrap_err()).unwrap(),
                            serde_json::to_value(expected).unwrap());
                    }
                }
                assert!(!actor.handle_completion(
                    "pinned-front".into(), old_task.epoch, &old_task.identity,
                    ok_end_turn(0, None), None,
                ).await, "late completion cannot acquire the cancelled front");
                assert!(matches!(response.try_recv(), Err(TryRecvError::Closed)));
                assert!(matches!(next_response.try_recv(), Err(TryRecvError::Empty)));
                {
                    let state = actor.state.lock().await;
                    assert!(state.running_task.is_none());
                    assert_eq!(state.pending_inputs.len(), 1);
                    let next = state.pending_inputs.front().unwrap();
                    assert_eq!(next.prompt_id, "queued-next");
                    assert_eq!(next.queue_meta, next_meta);
                    assert_eq!(serde_json::to_value(&next.prompt_blocks).unwrap(), next_blocks);
                    assert_eq!(next.client_identifier.as_deref(), Some("next-client"));
                    assert!(matches!(next.input_origin.as_prompt_origin(), crate::session::PromptOrigin::User));
                }
                assert!(actor.current_prompt_id.lock().unwrap().is_none());
                assert_eq!(report.commit(), super::super::turn_report_slot::CommitOutcome::Reported);
                assert_eq!(actor.tool_context.goal_loop_active_gate.load(Ordering::Relaxed), goal_was_active);
                while let Ok(event) = subagent_events.try_recv() {
                    use xai_grok_tools::implementations::grok_build::task::types::SubagentEvent;
                    assert!(!matches!(event,
                        SubagentEvent::Outstanding(_)
                            | SubagentEvent::ClearUsageNotApplied(_)
                            | SubagentEvent::MarkUsageNotApplied(_)
                    ), "NoTask cannot finalize a task's usage");
                }
                assert_eq!(terminal_attempts.load(Ordering::SeqCst), 1);
                assert_eq!(settlements.load(Ordering::SeqCst),
                    usize::from(matches!(commit, NoTaskTerminalCommit::Committed)));
                let reopened = crate::session::storage::jsonl::JsonlStorageAdapter::with_explicit_session_dir(
                    root.path().to_path_buf(),
                );
                let restored = reopened.load_session(&actor.session_info).await.unwrap();
                let terminals: Vec<_> = restored.updates.iter().filter(|update| {
                    matches!(update, SessionUpdate::Xai(notification)
                        if matches!(&notification.update, XaiSessionUpdate::TurnCompleted { .. }))
                }).collect();
                assert_eq!(terminals.len(), match commit {
                    NoTaskTerminalCommit::Committed | NoTaskTerminalCommit::AckLost => 1,
                    NoTaskTerminalCommit::WriteFailed => 0,
                });
            }).await.expect("bounded pinned NoTask cancel durability");
        })
        .await;
}

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
            matches!(update, SessionUpdate::Xai(notification) if matches!(&notification.update, XaiSessionUpdate::TurnCompleted { prompt_id, .. } if prompt_id == "durable-caller"))
        }).count(), 1);
    }).await;
}

#[tokio::test]
async fn failed_or_lost_terminal_ack_never_returns_caller_success() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for lose_ack in [false, true] {
                let (gateway_tx, _gateway_rx) = tokio::sync::mpsc::unbounded_channel();
                let (persistence_tx, mut persistence_rx) = tokio::sync::mpsc::unbounded_channel();
                let (actor, _) = create_test_actor_with_chat_persistence(
                    0,
                    256_000,
                    85,
                    gateway_tx,
                    persistence_tx,
                    Box::new(xai_chat_state::NullChatPersistence),
                )
                .await;
                let (input, mut response) =
                    input_with_origin_rx("failed-caller", crate::session::PromptOrigin::User);
                let task = running_task_stub("failed-caller");
                let epoch = task.epoch;
                let identity = task.identity.clone();
                {
                    let mut state = actor.state.lock().await;
                    state.pending_inputs.push_back(input);
                    state.running_task = Some(task);
                }
                let complete = actor.handle_completion(
                    "failed-caller".into(),
                    epoch,
                    &identity,
                    ok_end_turn(0, None),
                    None,
                );
                let reject = async {
                    loop {
                        if let PersistenceMsg::AppendUpdateDurablyAndAck { respond_to, .. } =
                            persistence_rx.recv().await.unwrap()
                        {
                            assert!(matches!(
                                response.try_recv(),
                                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
                            ));
                            if lose_ack {
                                drop(respond_to);
                            } else {
                                respond_to
                                    .send(Err(
                                        crate::session::storage::AppendUpdateError::NotCommitted(
                                            std::io::Error::new(
                                                std::io::ErrorKind::StorageFull,
                                                "terminal storage full",
                                            ),
                                        ),
                                    ))
                                    .unwrap();
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
                    assert!(
                        !matches!(message, PersistenceMsg::FinishDeliveryTurn { .. }),
                        "failed terminal must retain completion ownership"
                    );
                }
            }
        })
        .await;
}
