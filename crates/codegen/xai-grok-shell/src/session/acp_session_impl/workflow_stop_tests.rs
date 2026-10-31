use super::*;
use crate::session::acp_session::cancel::{StopPolicyCommit, WakeReengagement};
use crate::session::completion_delivery::{CompletionPayloadSource, CompletionWakePolicy, SuppressedCompletionSource};
use crate::session::workflow::{
    manager::{LaunchSpec, WorkflowManager},
    notify::WorkflowNotifySender,
    registry::resolve_inline,
    store::{RestoredWorkflowRun, WorkflowRunManifest, WorkflowRunStore},
    tracker::{WorkflowRunStatus, WorkflowTracker},
};
use xai_chat_state::completion::{CompletionIdentity, TerminalCompletionId};
use xai_grok_tools::implementations::grok_build::task::types::{SubagentEvent, SubagentResult, SubagentState};

async fn install_manager(
    actor: &SessionActor,
    directory: &std::path::Path,
    persistence: mpsc::UnboundedSender<PersistenceMsg>,
    commands: mpsc::UnboundedSender<crate::session::SessionCommand>,
    children: mpsc::UnboundedSender<SubagentEvent>,
    restored: Vec<RestoredWorkflowRun>,
) -> Arc<std::sync::atomic::AtomicUsize> {
    let (store, states) = WorkflowRunStore::from_restored(
        Some(directory.to_path_buf()), persistence.clone(), restored,
    );
    let tracker = Arc::new(parking_lot::Mutex::new(WorkflowTracker::from_snapshot(states)));
    let (gateway, _notifications) = mpsc::unbounded_channel();
    let notify = WorkflowNotifySender::new(
        actor.session_info.id.clone(), xai_acp_lib::AcpAgentGatewaySender::new(gateway),
        persistence, store.clone(),
    );
    let (host_children, mut host_requests) = mpsc::unbounded_channel();
    tokio::task::spawn_local(async move {
        while let Some(request) = host_requests.recv().await {
            match request {
                SubagentEvent::Cancel(request) => {
                    let _ = request.respond_to.send(Ok(
                        xai_grok_tools::implementations::grok_build::task::types::SubagentCancelOutcome::NotFound,
                    ));
                }
                other => { children.send(other).unwrap(); }
            }
        }
    });
    let active_work = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    *actor.workflow_manager.lock().await = WorkflowManager::new(
        actor.session_info.id.to_string(), Some(directory.to_path_buf()), directory.to_path_buf(),
        tracker, active_work.clone(), store, notify, host_children, Arc::new(|_, _, _| {}), commands,
        Default::default(), 4, Default::default(),
    );
    active_work
}

async fn persist_manifest(
    persistence: &mpsc::UnboundedSender<PersistenceMsg>,
    manifest: &WorkflowRunManifest,
) {
    let (respond_to, response) = oneshot::channel();
    persistence.send(PersistenceMsg::WorkflowRunStateAndAck {
        manifest: manifest.clone(), respond_to,
    }).unwrap();
    bounded(response).await.unwrap().unwrap();
}

fn spec(resume_run_id: Option<String>) -> LaunchSpec {
    LaunchSpec {
        objective: "workflow Stop authority".into(), args: serde_json::json!({}),
        agent_budget: Some(10), effort: None, resume_run_id,
    }
}

fn identity(state: &crate::session::workflow::tracker::WorkflowRunState) -> CompletionIdentity {
    TerminalCompletionId::Workflow { run_id: state.run_id.clone(), revision: state.revision }.into()
}

#[derive(Clone, Copy)]
enum Custody { Active, Retiring }

#[tokio::test(flavor = "current_thread")]
async fn real_workflow_stop_seals_episode_before_unknown_final_revision_and_survives_cold_restore() {
    tokio::task::LocalSet::new().run_until(async {
        for (custody, lose_stop_ack, lose_terminal_ack) in [
            (Custody::Active, false, false), (Custody::Active, true, true),
            (Custody::Retiring, false, true), (Custody::Retiring, true, false),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let info = crate::session::info::Info { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
            let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
            let owner = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (proxy, mut requests) = mpsc::unbounded_channel();
            let (terminal_tx, mut terminals) = mpsc::unbounded_channel();
            let (seal_tx, mut seals) = mpsc::unbounded_channel();
            let durable_manifest = persistence.handle.tx.clone();
            let durable = persistence.handle.tx;
            let proxy_owner = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(async move {
                let mut first_seal = true;
                while let Some(request) = requests.recv().await {
                    match request {
                        PersistenceMsg::WorkflowRunStateAndAck { manifest, respond_to } => {
                            terminal_tx.send((manifest, respond_to)).unwrap();
                        }
                        PersistenceMsg::SealHumanStoppedCompletions { ids, workflow_episodes, respond_to } => {
                            let (committed, receive) = oneshot::channel();
                            durable.send(PersistenceMsg::SealHumanStoppedCompletions { ids, workflow_episodes: workflow_episodes.clone(), respond_to: committed }).unwrap();
                            let result = receive.await.unwrap();
                            if first_seal {
                                first_seal = false;
                                seal_tx.send((workflow_episodes, respond_to, result)).unwrap();
                            } else {
                                let _ = respond_to.send(result);
                            }
                        }
                        other => { durable.send(other).unwrap(); }
                    }
                }
            }));
            let (gateway, _notifications) = mpsc::unbounded_channel();
            let actor = Arc::new(super::super::super::super::support::create_test_actor(0, 256_000, 85, gateway, proxy.clone()).await);
            let (commands, mut command_rx) = mpsc::unbounded_channel();
            let (children, mut child_rx) = mpsc::unbounded_channel();
            let work = install_manager(&actor, directory.path(), proxy.clone(), commands, children, Vec::new()).await;
            let script = "let meta = #{ name: \"old-episode\", description: \"retained result\" };\nlet r = agent(\"work\"); complete(r.output);";
            let (run_id, mut outcome) = actor.workflow_manager.lock().await.launch(resolve_inline(script.into()).unwrap(), spec(None)).unwrap();
            let SubagentEvent::Spawn(child) = bounded(child_rx.recv()).await.unwrap() else { panic!("expected real workflow host spawn"); };
            let mut child = Some(child);
            let mut complete_child = || {
                let child = child.take().unwrap();
                let id = child.id.clone();
                child.result_tx.send(SubagentResult {
                    state: SubagentState::Completed, output: Arc::from("original workflow result"),
                    subagent_id: id.clone(), child_session_id: id, ..Default::default()
                }).unwrap();
            };
            let mut held_terminal = match custody {
                Custody::Active => {
                    assert_eq!(actor.workflow_tracker().await.lock().get(&run_id).unwrap().status, WorkflowRunStatus::Active);
                    None
                }
                Custody::Retiring => {
                    complete_child();
                    let terminal = bounded(terminals.recv()).await.unwrap();
                    assert_eq!(terminal.0.state.status, WorkflowRunStatus::Complete);
                    assert!(!actor.workflow_manager.lock().await.pause(&run_id));
                    Some(terminal)
                }
            };
            let before = actor.workflow_tracker().await.lock().get(&run_id).unwrap();
            let session = actor.clone();
            let stopping = tokio::task::spawn_local(async move {
                session.cancel_running_task(crate::session::CancelOptions {
                    trigger: Some(crate::session::CancelTrigger::CtrlC), user_initiated: true, ..Default::default()
                }).await
            });
            let (episodes, stop_ack, result) = bounded(seals.recv()).await.unwrap();
            result.unwrap();
            assert!(episodes.contains(&before.episode()));
            assert!(!stopping.is_finished(), "Stop must await the actual durable seal ACK");
            assert!(matches!(outcome.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            assert!(work.load(std::sync::atomic::Ordering::Acquire) > 0, "Stop is not a drain claim");
            let sealed = actor.persisted_completion_state().await.unwrap();
            assert_eq!(sealed.wake_policy, CompletionWakePolicy::HumanStopped);
            assert!(sealed.stopped_workflow_episodes.contains(&before.episode()));
            let after = actor.workflow_tracker().await.lock().get(&run_id).unwrap();
            assert_eq!((after.status, after.revision), (before.status, before.revision));
            if lose_stop_ack { drop(stop_ack); } else { stop_ack.send(Ok(())).unwrap(); }
            let stopped = bounded(stopping).await.unwrap();
            match stopped.policy {
                StopPolicyCommit::Committed => { assert!(!lose_stop_ack); assert!(stopped.settled); }
                StopPolicyCommit::Failed(error) => {
                    assert!(lose_stop_ack);
                    assert!(!stopped.settled);
                    assert_eq!(error.to_string(), "Stop suppression commit acknowledgement lost");
                }
                StopPolicyCommit::NotRequired => panic!("explicit Stop requires a durable barrier"),
            }
            assert!(actor.state.lock().await.notifications_suppressed);
            assert!(actor.tool_context.task_wake_suppressed.as_ref().unwrap().get());
            let switched_storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            let switched_policy = switched_storage.recover_subagent_completions(&info).await.unwrap().completion_delivery.wake_policy;
            assert_eq!(switched_policy, CompletionWakePolicy::HumanStopped);
            let switched_persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), switched_storage);
            let switched_owner = tokio_util::task::AbortOnDropHandle::new(switched_persistence.task);
            let (gateway, _notifications) = mpsc::unbounded_channel();
            let mut switched = super::super::super::super::support::create_test_actor(0, 256_000, 85, gateway, switched_persistence.handle.tx).await;
            switched.workflow_manager = actor.workflow_manager.clone();
            switched.restore_completion_wake_policy(switched_policy).await;
            assert!(switched.state.lock().await.notifications_suppressed);
            assert!(switched.tool_context.task_wake_suppressed.as_ref().unwrap().get());
            drop(actor);
            let actor = Arc::new(switched);
            bounded(actor.clear_wake_suppression(WakeReengagement::NewHumanMessage)).await.unwrap();
            assert_eq!(actor.persisted_completion_state().await.unwrap().stopped_workflow_episodes.iter().filter(|episode| **episode == before.episode()).count(), 1);
            if matches!(custody, Custody::Active) { complete_child(); }
            let (original, original_ack) = match held_terminal.take() {
                Some(terminal) => terminal,
                None => bounded(terminals.recv()).await.unwrap(),
            };
            if matches!(custody, Custody::Active) {
                assert!(!actor.persisted_completion_state().await.unwrap().is_suppressed(&identity(&original.state)), "the real final revision was not enrolled at Stop");
                let runner = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(std::future::pending::<()>()));
                let epoch = actor.turn_report.start_next_turn();
                actor.state.lock().await.running_task = Some(AgentTask::new_at_epoch("new-human", epoch, runner.abort_handle()));
                bounded(actor.drain_between_turn_workflow_completions(false)).await;
                assert!(actor.persisted_completion_state().await.unwrap().receipts.is_empty(), "a new Human turn cannot absorb an unenrolled old episode reminder");
                actor.state.lock().await.running_task.take();
                actor.turn_report.release_aborted(epoch);
            }
            persist_manifest(&durable_manifest, &original).await;
            assert_eq!(original.state.status, WorkflowRunStatus::Complete);
            assert_eq!(original.state.result_summary.as_deref(), Some("original workflow result"));
            assert_eq!(original.state.episode(), before.episode());
            let final_manifest = if lose_terminal_ack {
                drop(original_ack);
                let (manifest, final_ack) = bounded(terminals.recv()).await.unwrap();
                persist_manifest(&durable_manifest, &manifest).await;
                assert_eq!(manifest.state.status, WorkflowRunStatus::Interrupted);
                assert!(manifest.state.revision > original.state.revision);
                assert_eq!(manifest.state.episode(), before.episode());
                assert!(manifest.state.pause_message.as_deref().unwrap().contains("acknowledgement"));
                final_ack.send(Ok(())).unwrap();
                manifest
            } else {
                original_ack.send(Ok(())).unwrap();
                original.clone()
            };
            match bounded(outcome).await.unwrap() {
                xai_workflow::WorkflowOutcome::Completed { result } => {
                    assert!(!lose_terminal_ack);
                    assert_eq!(result, serde_json::json!("original workflow result"));
                }
                xai_workflow::WorkflowOutcome::Failed { error } => {
                    assert!(lose_terminal_ack);
                    assert!(error.contains("acknowledgement"));
                }
                other => panic!("unexpected actual workflow outcome: {other:?}"),
            }
            let crate::session::SessionCommand::WorkflowCompletionTurn { run_id: published, revision } = bounded(command_rx.recv()).await.unwrap() else { panic!("expected actual manager publication"); };
            assert_eq!((published.clone(), revision), (run_id.clone(), final_manifest.state.revision));
            let (completion_tx, _completions) = mpsc::unbounded_channel();
            bounded(actor.clone().handle_workflow_completion_turn(completion_tx, published, revision)).await;
            assert!(actor.state.lock().await.running_task.is_none());
            assert!(actor.state.lock().await.pending_inputs.is_empty());
            assert!(actor.workflow_tracker().await.lock().is_unreported_completion(&run_id, revision), "suppression must not forge reporting");
            let persisted = actor.persisted_completion_state().await.unwrap();
            for manifest in [&original, &final_manifest] {
                assert!(persisted.is_suppressed(&identity(&manifest.state)));
                let denied = storage.append_completion_commit_aware(&info,
                    &xai_chat_state::CompletionProjection::new(identity(&manifest.state), "forbidden-old-inference".into(), "old workflow".into()),
                    &xai_chat_state::InputAdmission {
                        parent_task: xai_chat_state::ParentTask::Unknown,
                        cancellation: tokio_util::sync::CancellationToken::new(),
                    },
                ).await;
                assert!(matches!(denied, Err(xai_chat_state::StrictAppendError::NotCommitted(error))
                    if error.kind() == std::io::ErrorKind::PermissionDenied));
            }
            assert!(persisted.obligations.iter().any(|obligation| matches!(obligation,
                CompletionObligation::Suppressed { source: SuppressedCompletionSource::Enrolled(payload), .. }
                    if matches!(&payload.source, CompletionPayloadSource::Workflow(run)
                        if run.revision == original.state.revision && run.result_summary.as_deref() == Some("original workflow result")))));
            assert!(persisted.receipts.is_empty());
            let (respond_to, response) = oneshot::channel();
            durable_manifest.send(PersistenceMsg::FlushAndAck { respond_to }).unwrap();
            bounded(response).await.unwrap().unwrap();
            let visible = storage.load_session(&info).await.unwrap();
            assert!(visible.updates.iter().any(|update| matches!(update,
                crate::session::storage::SessionUpdate::Xai(notification)
                    if matches!(&notification.update,
                        crate::extensions::notification::SessionUpdate::WorkflowUpdated {
                            run_id: id, revision, result_summary, pause_message, ..
                        } if id == &run_id && *revision == final_manifest.state.revision
                            && result_summary == &final_manifest.state.result_summary
                            && pause_message == &final_manifest.state.pause_message))),
                "the real final completion remains in the user-visible update stream");
            drop(actor);
            drop(switched_owner);
            drop(proxy_owner);
            drop(owner);
            let reopened = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            let loaded = reopened.load_session_without_updates(&info).await.unwrap();
            let recovered = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
            assert_eq!(recovered.wake_policy, CompletionWakePolicy::Automatic);
            assert!(recovered.is_suppressed(&identity(&final_manifest.state)));
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), reopened.clone());
            let _owner = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (gateway, _notifications) = mpsc::unbounded_channel();
            let actor = Arc::new(super::super::super::super::support::create_test_actor(0, 256_000, 85, gateway, persistence.handle.tx.clone()).await);
            let (commands, mut command_rx) = mpsc::unbounded_channel();
            let (children, _child_rx) = mpsc::unbounded_channel();
            install_manager(&actor, directory.path(), persistence.handle.tx, commands, children, loaded.workflow_runs).await;
            actor.restore_completion_wake_policy(recovered.wake_policy).await;
            let (completion_tx, _completions) = mpsc::unbounded_channel();
            bounded(actor.clone().handle_workflow_completion_turn(completion_tx.clone(), run_id, revision)).await;
            assert!(actor.state.lock().await.running_task.is_none());
            let (fresh, outcome) = actor.workflow_manager.lock().await.launch(resolve_inline("let meta = #{ name: \"fresh-episode\", description: \"new Human authority\" }; complete(\"fresh result\");".into()).unwrap(), spec(None)).unwrap();
            assert!(matches!(bounded(outcome).await.unwrap(), xai_workflow::WorkflowOutcome::Completed { .. }));
            let crate::session::SessionCommand::WorkflowCompletionTurn { run_id, revision } = bounded(command_rx.recv()).await.unwrap() else { panic!("expected fresh workflow completion"); };
            assert_eq!(run_id, fresh);
            let fresh_state = actor.workflow_tracker().await.lock().get(&run_id).unwrap();
            assert_eq!(fresh_state.revision, revision);
            assert!(!actor.persisted_completion_state().await.unwrap().is_suppressed(&identity(&fresh_state)));
            bounded(actor.clone().handle_workflow_completion_turn(completion_tx, run_id.clone(), revision)).await;
            assert_eq!(actor.state.lock().await.running_prompt_id(), Some(format!("workflow-completed-{run_id}-{revision}").as_str()));
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn cold_restored_stopped_run_resumes_with_its_real_next_execution_epoch() {
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::info::Info { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
        let owner = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (gateway, _notifications) = mpsc::unbounded_channel();
        let actor = super::super::super::super::support::create_test_actor(0, 256_000, 85, gateway, persistence.handle.tx.clone()).await;
        let (commands, _command_rx) = mpsc::unbounded_channel();
        let (children, _child_rx) = mpsc::unbounded_channel();
        install_manager(&actor, directory.path(), persistence.handle.tx, commands, children, Vec::new()).await;
        let script = "let meta = #{ name: \"resume-episode\", description: \"resume real journal\" }; await_user(\"user\", \"first pause\"); await_user(\"user\", \"second pause\"); complete(\"resumed result\");";
        let (run_id, outcome) = actor.workflow_manager.lock().await.launch(resolve_inline(script.into()).unwrap(), spec(None)).unwrap();
        assert!(matches!(bounded(outcome).await.unwrap(), xai_workflow::WorkflowOutcome::Paused { .. }));
        let (_, outcome) = actor.workflow_manager.lock().await.launch(resolve_inline(script.into()).unwrap(), spec(Some(run_id.clone()))).unwrap();
        assert!(matches!(bounded(outcome).await.unwrap(), xai_workflow::WorkflowOutcome::Paused { .. }));
        let episode = actor.workflow_tracker().await.lock().get(&run_id).unwrap().episode();
        assert_eq!(episode.execution_epoch, 1);
        let stopped = bounded(actor.cancel_running_task(crate::session::CancelOptions {
            trigger: Some(crate::session::CancelTrigger::CtrlC), user_initiated: true, ..Default::default()
        })).await;
        assert!(stopped.settled);
        drop(actor);
        drop(owner);
        let reopened = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
        let loaded = reopened.load_session_without_updates(&info).await.unwrap();
        let recovered = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
        assert_eq!(recovered.wake_policy, CompletionWakePolicy::HumanStopped);
        assert!(recovered.stopped_workflow_episodes.contains(&episode));
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), reopened.clone());
        let _owner = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (gateway, _notifications) = mpsc::unbounded_channel();
        let actor = Arc::new(super::super::super::super::support::create_test_actor(0, 256_000, 85, gateway, persistence.handle.tx.clone()).await);
        let (commands, mut command_rx) = mpsc::unbounded_channel();
        let (children, _child_rx) = mpsc::unbounded_channel();
        install_manager(&actor, directory.path(), persistence.handle.tx, commands, children, loaded.workflow_runs).await;
        actor.restore_completion_wake_policy(recovered.wake_policy).await;
        actor.clear_wake_suppression(WakeReengagement::GoalResume).await.unwrap();
        assert_eq!(actor.persisted_completion_state().await.unwrap().wake_policy, CompletionWakePolicy::HumanStopped);
        bounded(actor.clear_wake_suppression(WakeReengagement::NewHumanMessage)).await.unwrap();
        let (same_run, outcome) = actor.workflow_manager.lock().await.launch(resolve_inline(script.into()).unwrap(), spec(Some(run_id.clone()))).unwrap();
        assert_eq!(same_run, run_id);
        assert!(matches!(bounded(outcome).await.unwrap(), xai_workflow::WorkflowOutcome::Completed { .. }));
        let state = actor.workflow_tracker().await.lock().get(&run_id).unwrap();
        assert_eq!(state.execution_epoch, episode.execution_epoch + 1);
        let disk: WorkflowRunManifest = serde_json::from_slice(&std::fs::read(directory.path().join("workflows").join(&run_id).join("state.json")).unwrap()).unwrap();
        assert_eq!(disk.state.episode(), state.episode());
        let delivery = actor.persisted_completion_state().await.unwrap();
        assert!(delivery.stopped_workflow_episodes.contains(&episode));
        assert!(!delivery.stopped_workflow_episodes.contains(&state.episode()));
        assert!(!delivery.is_suppressed(&identity(&state)));
        let crate::session::SessionCommand::WorkflowCompletionTurn { run_id, revision } = bounded(command_rx.recv()).await.unwrap() else { panic!("expected resumed episode completion"); };
        assert_eq!(revision, state.revision);
        let (completion_tx, _completions) = mpsc::unbounded_channel();
        bounded(actor.clone().handle_workflow_completion_turn(completion_tx, run_id.clone(), revision)).await;
        assert_eq!(actor.state.lock().await.running_prompt_id(), Some(format!("workflow-completed-{run_id}-{revision}").as_str()));
    }).await;
}
