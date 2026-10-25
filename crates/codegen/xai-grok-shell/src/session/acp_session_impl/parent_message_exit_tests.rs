use super::*;
use super::boundary_tests::{actual_shutdown, persist_peer, stop_persistence};
use crate::session::delivery_receiver::{DeliveryExecution, DeliveryProvenance, ParentMessageOperation, ReceiverResume};
use crate::session::storage::{AtomicWriteFault, fail_next_atomic_write};

#[derive(Clone, Copy, Debug)]
enum CancelCut { Acknowledged, Precommit, Postrename, LostAck }

#[derive(Clone, Copy, Debug)]
enum Exit { Shutdown, CommandsClosed }

#[derive(Clone, Copy, Debug)]
enum ParentRoute { Queue, Recovered, Fallback }

async fn fixture(cut: CancelCut, barrier: ReceiverBarrier, projection: ReceiverBarrier) -> ReceiverFixture {
    let directory = tempfile::tempdir().unwrap();
    let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    let storage = match cut {
        CancelCut::Acknowledged | CancelCut::LostAck => storage,
        CancelCut::Precommit => storage.with_parent_settlement_probe(|| { fail_next_atomic_write(AtomicWriteFault::Write); Ok(()) }),
        CancelCut::Postrename => storage.with_parent_settlement_probe(|| { fail_next_atomic_write(AtomicWriteFault::ParentSync); Ok(()) }),
    };
    let ack = match cut {
        CancelCut::LostAck => ReceiverSettlementAck::Lose,
        CancelCut::Acknowledged | CancelCut::Precommit | CancelCut::Postrename => ReceiverSettlementAck::Forward,
    };
    receiver_with_exit_cuts(directory, storage, barrier, ack, projection).await
}

async fn exit(receiver: &mut ReceiverFixture, exit: Exit) {
    match exit {
        Exit::Shutdown => actual_shutdown(receiver, crate::session::ShutdownKind::CancelRunningTurn).await,
        Exit::CommandsClosed => {
            let (_, empty) = mpsc::unbounded_channel();
            let events = std::mem::replace(&mut receiver.events, empty);
            let (commands, command_rx) = mpsc::unbounded_channel();
            drop(commands);
            let (_chat, chat_rx) = mpsc::unbounded_channel();
            bounded(super::super::super::run_session(
                receiver.actor.clone(), command_rx, chat_rx, events, None,
                Arc::new(parking_lot::Mutex::new(xai_grok_workspace::file_system::CodebaseIndexManager::new())),
                std::path::PathBuf::from("/tmp"), crate::session::fs_watch::FsWatchCapabilities::none(),
            )).await;
        }
    }
}

fn assert_uncertain(result: crate::session::commands::PromptTurnResult) {
    let error = result.expect_err("unacknowledged parent teardown cannot publish Cancelled");
    assert_eq!(serde_json::to_value(error).unwrap()["data"],
        "parent admission outcome uncertain after unacknowledged teardown");
}

async fn assert_cut(receiver: &mut ReceiverFixture, cut: CancelCut) {
    let observed = bounded(receiver.settlement_results.recv()).await.unwrap();
    assert_eq!(observed, match cut {
        CancelCut::Acknowledged | CancelCut::LostAck => SettlementObservation::Committed,
        CancelCut::Precommit => SettlementObservation::Fault(AtomicWriteFault::Write),
        CancelCut::Postrename => SettlementObservation::Fault(AtomicWriteFault::ParentSync),
    });
    assert!(matches!(receiver.settlement_results.try_recv(), Err(mpsc::error::TryRecvError::Empty)), "failed teardown must not silently retry");
    assert_eq!(receiver.actor.state.lock().await.message_delivery.teardown, match cut {
        CancelCut::Acknowledged => ParentTeardownStatus::Settled,
        CancelCut::Precommit | CancelCut::Postrename | CancelCut::LostAck => ParentTeardownStatus::Unsettled,
    });
}

async fn parked_user(receiver: &ReceiverFixture) -> (oneshot::Sender<()>, mpsc::UnboundedReceiver<super::super::super::turn_task::TurnCompletionMsg>) {
    let (release, start_gate) = oneshot::channel();
    let (completion, completed) = mpsc::unbounded_channel();
    let mut state = receiver.actor.state.lock().await;
    state.running_task.take().unwrap().abort();
    state.pending_inputs.pop_front().unwrap();
    state.pending_inputs.push_front(super::super::super::support::user_item("held-tool-turn", "literal user work"));
    let epoch = receiver.actor.turn_report.start_next_turn();
    state.running_task = Some(AgentTask::new_prompt(receiver.actor.clone(), super::super::super::turn_task::TurnInputRequest {
        prompt_id: "held-tool-turn".into(), input_origin: InputOrigin::new(PromptOrigin::User),
        prompt_blocks: vec![acp::ContentBlock::Text(acp::TextContent::new("literal user work"))],
        prompt_mode: PromptMode::Agent, trace_gcs_config: None, artifact_tracker: None,
        client_identifier: None, screen_mode: None, verbatim: true, send_now: false,
        json_schema: None, persist_ack: None, parsed_prompt_tx: None, traceparent: None,
        start_gate: Some(start_gate),
    }, epoch, completion));
    (release, completed)
}

#[tokio::test(flavor = "current_thread")]
async fn actual_shutdown_and_closed_channel_preserve_unacknowledged_parent_uncertainty() {
    tokio::task::LocalSet::new().run_until(async {
        for ending in [Exit::Shutdown, Exit::CommandsClosed] {
            for cut in [CancelCut::Acknowledged, CancelCut::Precommit, CancelCut::Postrename, CancelCut::LostAck] {
                let mut receiver = fixture(cut, ReceiverBarrier::HoldNext, ReceiverBarrier::Forward).await;
                let (release_user, mut completed) = parked_user(&receiver).await;
                let (token, handle, task_id) = {
                    let state = receiver.actor.state.lock().await;
                    let task = state.running_task.as_ref().unwrap();
                    (task.admission.clone(), task.handle.clone(), task.identity.as_ref().clone())
                };
                let Producer { delivery, reply, outcome, _coordinator, _spawn, .. } = producer().await;
                let message_id = delivery.message().message_id.clone();
                let (admitting, mut receipts) = dispatch(receiver.actor.clone(), delivery, reply);
                let (admission, proof, ack) = bounded(receiver.acknowledgements.recv()).await.unwrap();
                assert_eq!(admission.effective, ParentMessageOperation::Steer { prompt_id: "held-tool-turn".into(), parent_task: xai_chat_state::ParentTask::Instance { id: task_id } });
                ack.send(proof).unwrap();
                bounded(admitting).await.unwrap();
                assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::Accepted { message_id: message_id.clone() });
                let receipt = receipts.recv().await.unwrap();
                let (_peer, mut peer_receipt, peer_task) = persist_peer(&mut receiver).await;
                let draining = tokio::task::spawn_local({ let actor = receiver.actor.clone(); async move { actor.drain_parent_messages_at_safe_point().await } });
                let (flushed, flush_ack) = bounded(receiver.barriers.recv()).await.unwrap();
                assert!(flushed.is_ok());
                exit(&mut receiver, ending).await;
                assert_cut(&mut receiver, cut).await;
                assert!(token.is_cancelled());
                assert!(bounded(completed.recv()).await.is_none(), "the actual task must abort, not finish later work");
                assert!(handle.is_finished());
                assert!(release_user.send(()).is_err());
                flush_ack.send(flushed).unwrap();
                assert!(!bounded(draining).await.unwrap());
                let result = bounded(receipt.result).await.unwrap();
                match cut {
                    CancelCut::Acknowledged => {
                        let error = result.unwrap_err();
                        assert_eq!(serde_json::to_value(error).unwrap()["data"], "active-message owner ended before delivery completed");
                    }
                    CancelCut::Precommit | CancelCut::Postrename | CancelCut::LostAck => assert_uncertain(result),
                }
                assert!(matches!(peer_receipt.result.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                assert!(receiver.actor.state.lock().await.running_task.is_none());
                assert!(persisted_bindings(&receiver).await.is_empty());
                stop_persistence(&mut receiver).await;
                let reopened = JsonlStorageAdapter::with_explicit_session_dir(receiver._directory.path().to_path_buf());
                let (_lease, summary) = reopened.claim_receiver(&receiver.actor.session_info).await.unwrap();
                let record = &summary.completion_delivery.receiver.records[&admission.delivery_id()];
                match cut {
                    CancelCut::Precommit => assert!(matches!(&record.execution, DeliveryExecution::Queued { payload, .. } if payload == "claim-bound text stays behind the tool boundary")),
                    CancelCut::Acknowledged | CancelCut::Postrename | CancelCut::LostAck => assert!(matches!(record.execution, DeliveryExecution::Cancelled { .. })),
                }
                let peer = &summary.completion_delivery.receiver.records["parent-message/reused-prompt-peer"];
                assert!(matches!(peer.execution, DeliveryExecution::Queued { .. }));
                assert!(matches!(&peer.provenance, DeliveryProvenance::ParentMessage { admission }
                    if matches!(&admission.effective, ParentMessageOperation::Steer { parent_task, .. } if parent_task == &peer_task)));
                assert!(reopened.load_session(&receiver.actor.session_info).await.unwrap().chat_history.iter().all(|item| item.text_content() != "claim-bound text stays behind the tool boundary"));
            }
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn promoted_parent_shutdown_stops_exact_live_task_after_real_projection() {
    tokio::task::LocalSet::new().run_until(async {
        for route in [ParentRoute::Queue, ParentRoute::Recovered, ParentRoute::Fallback] {
            for ending in [Exit::Shutdown, Exit::CommandsClosed] {
                for cut in [CancelCut::Acknowledged, CancelCut::Precommit, CancelCut::Postrename, CancelCut::LostAck] {
                    let mut receiver = fixture(cut, ReceiverBarrier::Forward, ReceiverBarrier::HoldNext).await;
                    let operation = match route { ParentRoute::Queue | ParentRoute::Recovered => ActiveAgentMessageOperation::Queue, ParentRoute::Fallback => ActiveAgentMessageOperation::Steer };
                    let Producer { delivery, reply, outcome, _coordinator, _spawn, .. } = producer_with_operation(operation).await;
                    let message_id = delivery.message().message_id.clone();
                    let (admitting, mut receipts) = dispatch(receiver.actor.clone(), delivery, reply);
                    let (admission, proof, ack) = bounded(receiver.acknowledgements.recv()).await.unwrap();
                    let receipt = match route {
                        ParentRoute::Recovered => {
                            drop(proof); drop(ack);
                            bounded(admitting).await.unwrap();
                            assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::AdmissionUncertain);
                            assert!(receipts.recv().await.is_none());
                            receiver.actor.resume_durable_deliveries(ReceiverResume::Recovery { deliveries: vec![format!("parent-message/{message_id}")] }).await;
                            None
                        }
                        ParentRoute::Queue | ParentRoute::Fallback => {
                            ack.send(proof).unwrap();
                            bounded(admitting).await.unwrap();
                            assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::Accepted { message_id: message_id.clone() });
                            Some(receipts.recv().await.unwrap())
                        }
                    };
                    match route {
                        ParentRoute::Fallback => {
                            let outcome = receiver.actor.cancel_running_task(crate::session::CancelOptions {
                                cancel_subagents: true, kill_background_tasks: false,
                                history: crate::session::CancelHistoryDisposition::Keep,
                                trigger: None, user_initiated: false,
                            }).await;
                            assert!(outcome.settled);
                        }
                        ParentRoute::Queue | ParentRoute::Recovered => {
                            let mut state = receiver.actor.state.lock().await;
                            state.running_task.take().unwrap().abort();
                            state.pending_inputs.retain(|item| item.prompt_id != "held-tool-turn");
                        }
                    }
                    let (entered, parked) = oneshot::channel();
                    let (release_work, continue_work) = oneshot::channel();
                    receiver.actor.state.lock().await.message_delivery.projection_cut = ParentProjectionCut::Park { entered, release: continue_work };
                    let (completion, mut completed) = mpsc::unbounded_channel();
                    SessionActor::maybe_start_running_task(receiver.actor.clone(), completion).await;
                    let (_, projected, projection_ack) = bounded(receiver.projections.recv()).await.unwrap();
                    assert!(matches!(&projected, Ok(xai_chat_state::CompletionProjectionAck::Appended)));
                    let (token, handle, live_task) = {
                        let state = receiver.actor.state.lock().await;
                        let task = state.running_task.as_ref().unwrap();
                        assert_eq!(state.pending_inputs.front().unwrap().prompt_id, task.prompt_id);
                        assert_eq!(task.prompt_id, format!("parent-message-{message_id}"));
                        (task.admission.clone(), task.handle.clone(), xai_chat_state::ParentTask::Instance { id: task.identity.as_ref().clone() })
                    };
                    match &admission.effective {
                        ParentMessageOperation::Queue => {}
                        ParentMessageOperation::Steer { parent_task, .. } | ParentMessageOperation::Interject { parent_task, .. } => assert_ne!(parent_task, &live_task),
                    }
                    projection_ack.send(projected).unwrap();
                    bounded(parked).await.unwrap();
                    assert!(!token.is_cancelled());
                    assert!(!handle.is_finished());
                    let (_peer, mut peer_receipt, _) = persist_peer(&mut receiver).await;
                    exit(&mut receiver, ending).await;
                    assert_cut(&mut receiver, cut).await;
                    assert!(token.is_cancelled());
                    assert!(bounded(completed.recv()).await.is_none());
                    assert!(handle.is_finished());
                    assert!(release_work.send(()).is_err(), "parked work must be dropped by the exact task abort");
                    if let Some(receipt) = receipt {
                        assert_eq!(receipt.prompt_id, format!("parent-message-{message_id}"));
                        let result = bounded(receipt.result).await.unwrap();
                        match cut {
                            CancelCut::Acknowledged => assert_eq!(result.unwrap().stop_reason, acp::StopReason::Cancelled),
                            CancelCut::Precommit | CancelCut::Postrename | CancelCut::LostAck => assert_uncertain(result),
                        }
                    }
                    assert!(matches!(peer_receipt.result.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                    assert!(receiver.actor.state.lock().await.running_task.is_none());
                    assert!(!receiver.actor.state.lock().await.finalization_gate.is_active());
                    stop_persistence(&mut receiver).await;
                    let reopened = JsonlStorageAdapter::with_explicit_session_dir(receiver._directory.path().to_path_buf());
                    let (_lease, summary) = reopened.claim_receiver(&receiver.actor.session_info).await.unwrap();
                    let record = &summary.completion_delivery.receiver.records[&format!("parent-message/{message_id}")];
                    match cut {
                        CancelCut::Precommit => assert!(matches!(&record.execution, DeliveryExecution::Interrupted { parent_task, .. } if parent_task == &live_task)),
                        CancelCut::Acknowledged | CancelCut::Postrename | CancelCut::LostAck => assert!(matches!(record.execution, DeliveryExecution::Cancelled { .. })),
                    }
                    assert!(matches!(summary.completion_delivery.receiver.records["parent-message/reused-prompt-peer"].execution, DeliveryExecution::Queued { .. }));
                    let bindings = persisted_bindings(&receiver).await;
                    assert_eq!(bindings.len(), 1);
                    assert_eq!(bindings[0].delivery_id, format!("parent-message/{message_id}"));
                    assert_eq!(bindings[0].parent_task, live_task);
                    assert_eq!(bindings[0].turn_id, format!("parent-message-{message_id}"));
                    use sha2::Digest;
                    assert_eq!(bindings[0].payload_digest, format!("{:x}", sha2::Sha256::digest(b"claim-bound text stays behind the tool boundary")));
                    let loaded = reopened.load_session(&receiver.actor.session_info).await.unwrap();
                    let items: Vec<_> = loaded.chat_history.iter().filter(|item| item.text_content() == "claim-bound text stays behind the tool boundary").collect();
                    assert_eq!(items.len(), 1);
                    boundary_tests::assert_agent_model_projection(items[0]);
                }
            }
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn graceful_exit_preserves_promoted_parent_and_its_exact_running_front() {
    tokio::task::LocalSet::new().run_until(async {
        let mut receiver = fixture(CancelCut::Acknowledged, ReceiverBarrier::Forward, ReceiverBarrier::HoldNext).await;
        let Producer { delivery, reply, outcome, _coordinator, _spawn, .. } = producer_with_operation(ActiveAgentMessageOperation::Queue).await;
        let message_id = delivery.message().message_id.clone();
        let (admitting, mut receipts) = dispatch(receiver.actor.clone(), delivery, reply);
        let (_, proof, ack) = bounded(receiver.acknowledgements.recv()).await.unwrap();
        ack.send(proof).unwrap();
        bounded(admitting).await.unwrap();
        assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::Accepted { message_id: message_id.clone() });
        let mut receipt = receipts.recv().await.unwrap();
        {
            let mut state = receiver.actor.state.lock().await;
            state.running_task.take().unwrap().abort();
            state.pending_inputs.retain(|item| item.prompt_id != "held-tool-turn");
        }
        let (entered, parked) = oneshot::channel();
        let (release, continue_work) = oneshot::channel();
        receiver.actor.state.lock().await.message_delivery.projection_cut = ParentProjectionCut::Park { entered, release: continue_work };
        let (completion, mut completed) = mpsc::unbounded_channel();
        SessionActor::maybe_start_running_task(receiver.actor.clone(), completion).await;
        let (_, projected, ack) = bounded(receiver.projections.recv()).await.unwrap();
        assert!(matches!(&projected, Ok(xai_chat_state::CompletionProjectionAck::Appended)));
        ack.send(projected).unwrap();
        bounded(parked).await.unwrap();
        let identity = receiver.actor.state.lock().await.running_task.as_ref().unwrap().identity.clone();
        actual_shutdown(&mut receiver, crate::session::ShutdownKind::Graceful).await;
        {
            let state = receiver.actor.state.lock().await;
            let task = state.running_task.as_ref().unwrap();
            assert!(std::rc::Rc::ptr_eq(&identity, &task.identity));
            assert_eq!(state.pending_inputs.front().unwrap().prompt_id, task.prompt_id);
            assert!(!task.admission.is_cancelled());
            assert!(!task.handle.is_finished());
        }
        assert!(matches!(receipt.result.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
        assert!(matches!(receiver.settlement_results.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        let loaded = receiver.storage.load_session(&receiver.actor.session_info).await.unwrap();
        assert!(matches!(loaded.summary.completion_delivery.receiver.records[&format!("parent-message/{message_id}")].execution, DeliveryExecution::Processing { .. }));
        receiver.actor.state.lock().await.running_task.as_ref().unwrap().abort();
        assert!(bounded(completed.recv()).await.is_none());
        assert!(release.send(()).is_err());
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn actual_actor_drop_reports_uncertainty_without_cancelling_queued_intent() {
    tokio::task::LocalSet::new().run_until(async {
        for operation in [ActiveAgentMessageOperation::Steer, ActiveAgentMessageOperation::Interject, ActiveAgentMessageOperation::Queue] {
        let mut receiver = receiver().await;
        let Producer { delivery, reply, outcome, _coordinator, _spawn, .. } = producer_with_operation(operation).await;
        let message_id = delivery.message().message_id.clone();
        let (admitting, mut receipts) = dispatch(receiver.actor.clone(), delivery, reply);
        let (_, proof, ack) = bounded(receiver.acknowledgements.recv()).await.unwrap();
        ack.send(proof).unwrap();
        bounded(admitting).await.unwrap();
        assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::Accepted { message_id: message_id.clone() });
        let receipt = receipts.recv().await.unwrap();
        receiver.actor.state.lock().await.running_task.take().unwrap().abort();
        stop_persistence(&mut receiver).await;
        receiver._proxy.abort();
        assert!(bounded(&mut receiver._proxy).await.unwrap_err().is_cancelled());
        let ReceiverFixture { actor, storage, _directory, .. } = receiver;
        let info = actor.session_info.clone();
        assert_eq!(Arc::strong_count(&actor), 1, "observe actual final actor ownership, not a run-loop return");
        drop(actor);
        let observation = bounded(receipt.result).await;
        match operation {
            ActiveAgentMessageOperation::Steer | ActiveAgentMessageOperation::Interject => assert_uncertain(observation.unwrap()),
            ActiveAgentMessageOperation::Queue => assert!(observation.is_err(), "queue owner drop is lost observation, not durable Cancelled"),
        }
        let (_lease, summary) = storage.claim_receiver(&info).await.unwrap();
        assert!(matches!(summary.completion_delivery.receiver.records[&format!("parent-message/{message_id}")].execution, DeliveryExecution::Queued { .. }));
        assert!(storage.load_session(&info).await.unwrap().chat_history.iter().all(|item| item.text_content() != "claim-bound text stays behind the tool boundary"));
        }
    }).await;
}
