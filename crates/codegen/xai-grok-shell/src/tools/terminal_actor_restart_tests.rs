use super::*;
use crate::session::acp_session::support;
use crate::session::chat_persistence::ChannelChatPersistence;
use crate::session::completion_delivery::{CompletionEventPolicy, CompletionObligation};
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter};
use xai_grok_tools::computer::local::terminal::LocalTerminalBackend;
use xai_grok_tools::computer::types::{TerminalBackend, TerminalRunRequest};

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(15), future).await.expect("terminal actor restart rendezvous timed out")
}

#[tokio::test(flavor = "current_thread")]
async fn actual_bash_and_monitor_producer_bridge_actor_admission_replays_once_after_restart() {
    tokio::task::LocalSet::new().run_until(async {
        for kind in [TaskKind::Bash, TaskKind::Monitor] {
            let directory = tempfile::tempdir().unwrap();
            let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
            let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
            let persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (gateway_tx, gateway_rx) = mpsc::unbounded_channel();
            support::drain_gateway(gateway_rx);
            let (actor, events) = support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                gateway_tx, persistence.handle.tx.clone(), Box::new(ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
            actor.state.lock().await.running_task = Some(support::running_task_stub("busy-before-crash"));
            let actor = Arc::new(actor);
            let (session, actor_task) = support::spawn_completion_receiver(actor.clone(), events);
            let (mut config, gateway, _unused_persistence, _unused_commands) = make_test_config_full_raw();
            support::drain_gateway(gateway);
            config.session_id = info.id.clone();
            config.persistence = persistence.handle.clone();
            config.session_cmd_tx = session.clone();
            let bridge = spawn_notification_bridge(config);
            let terminal = LocalTerminalBackend::new_local(Default::default());
            let background = bounded(terminal.run_background(TerminalRunRequest {
                command: "printf restart-terminal-evidence".into(), working_directory: "/tmp".into(), env: Default::default(),
                timeout: std::time::Duration::from_secs(10), output_byte_limit: 4096,
                output_file: directory.path().join("terminal-output"), notification_handle: bridge,
                tool_call_id: format!("producer-{kind:?}"), display_command: None, auto_background_on_timeout: false,
                foreground_block_budget: None, kind, owner_session_id: Some(info.id.to_string()), description: None,
            })).await.unwrap();
            bounded(async {
                loop {
                    let (reply, response) = tokio::sync::oneshot::channel();
                    session.send(SessionCommand::GetToolOverrides { respond_to: reply }).unwrap();
                    response.await.unwrap();
                    if actor.state.lock().await.pending_inputs.iter().any(|input| matches!(input.input_origin.as_prompt_origin(), crate::session::PromptOrigin::TaskCompleted { task_id } if task_id == &background.task_id)) { break; }
                    tokio::task::yield_now().await;
                }
            }).await;
            let terminal_snapshot = bounded(terminal.get_task(&background.task_id)).await.unwrap();
            assert!(terminal_snapshot.completed);
            assert!(actor.chat_state_handle.snapshot().await.unwrap().conversation.iter().all(|item| !item.text_content().contains(&background.task_id)));
            drop(actor_task);
            drop(actor);
            drop(session);
            drop(terminal);
            drop(persistence_task);
            tokio::task::yield_now().await;

            let reopened = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            let recovered = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
            assert_eq!(recovered.obligations.len(), 1);
            assert!(recovered.receipts.is_empty(), "crash before safe-point projection must leave the producer obligation pending");
            let CompletionObligation::Pending { id, payload } = recovered.obligations[0].clone() else { panic!("terminal producer must restore typed pending admission") };
            assert!(matches!(payload.policy, CompletionEventPolicy::Automatic));
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), reopened.clone());
            let _persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (gateway_tx, gateway_rx) = mpsc::unbounded_channel();
            support::drain_gateway(gateway_rx);
            let (replacement, events) = support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                gateway_tx, persistence.handle.tx.clone(), Box::new(ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
            replacement.state.lock().await.running_task = Some(support::running_task_stub("replacement-caller"));
            let replacement = Arc::new(replacement);
            let (session, _actor_task) = support::spawn_completion_receiver(replacement.clone(), events);
            session.send(SessionCommand::ResumeTerminalCompletion { id: id.clone(), text: payload.model_text(&id), policy: payload.policy }).unwrap();
            let (reply, response) = tokio::sync::oneshot::channel();
            session.send(SessionCommand::GetToolOverrides { respond_to: reply }).unwrap();
            bounded(response).await.unwrap();
            assert!(bounded(replacement.drain_terminal_wakes_at_safe_point()).await);
            let snapshot = replacement.chat_state_handle.snapshot().await.unwrap();
            assert_eq!(snapshot.conversation.iter().filter(|item| item.text_content().contains(&background.task_id)).count(), 1);
            session.send(SessionCommand::ResumeTerminalCompletion { id: id.clone(), text: payload.model_text(&id), policy: payload.policy }).unwrap();
            let (reply, response) = tokio::sync::oneshot::channel();
            session.send(SessionCommand::GetToolOverrides { respond_to: reply }).unwrap();
            bounded(response).await.unwrap();
            assert!(!bounded(replacement.drain_terminal_wakes_at_safe_point()).await);
            let state = reopened.read_completion_state(&info).await.unwrap();
            assert_eq!(state.receipts.len(), 1);
            assert_eq!(state.receipts[0].completion, id);
            assert_eq!(replacement.chat_state_handle.snapshot().await.unwrap().conversation.iter().filter(|item| item.text_content().contains(&background.task_id)).count(), 1);
        }
    }).await;
}

fn assert_original_terminal_payload(
    id: &xai_chat_state::completion::CompletionIdentity,
    payload: &crate::session::completion_delivery::CompletionPayload,
    task_id: &str,
    kind: TaskKind,
    output: &str,
    output_file: &std::path::Path,
) {
    use xai_chat_state::completion::{CompletionIdentity, CompletionTaskKind, TerminalCompletionId};
    let task_kind = match kind { TaskKind::Bash => CompletionTaskKind::Bash, TaskKind::Monitor => CompletionTaskKind::Monitor };
    let expected: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: "test-actor".into(), task_id: task_id.into(), task_kind }.into();
    assert_eq!(id, &expected);
    assert!(matches!(payload.policy, CompletionEventPolicy::Automatic));
    let crate::session::completion_delivery::CompletionPayloadSource::Task(snapshot) = &payload.source else { panic!("real producer payload must retain the original task snapshot") };
    assert_eq!(snapshot.task_id, task_id);
    assert_eq!(snapshot.owner_session_id.as_deref(), Some("test-actor"));
    assert_eq!(snapshot.kind, kind);
    assert_eq!(snapshot.command, format!("printf {output}"));
    assert_eq!(snapshot.cwd, "/tmp");
    assert_eq!(snapshot.output, output);
    assert_eq!(snapshot.output_total_bytes, output.len());
    assert_eq!(snapshot.output_file.as_path(), output_file);
    assert_eq!(snapshot.exit_code, Some(0));
    assert_eq!(snapshot.signal, None);
    assert!(snapshot.completed && snapshot.end_time.is_some() && snapshot.is_backgrounded);
    assert!(!snapshot.block_waited && !snapshot.explicitly_killed && !snapshot.kill_result_delivered);
}

#[tokio::test(flavor = "current_thread")]
async fn actual_goal_active_bash_and_monitor_bridge_commits_policy_before_publication_and_restart() {
    tokio::task::LocalSet::new().run_until(async {
        for kind in [TaskKind::Bash, TaskKind::Monitor] {
            let directory = tempfile::tempdir().unwrap();
            let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
            let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
            let persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (gateway, gateway_rx) = mpsc::unbounded_channel();
            support::drain_gateway(gateway_rx);
            let (actor, events) = support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                gateway, persistence.handle.tx.clone(), Box::new(ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
            support::set_goal_harness_for_tests(&actor);
            actor.goal_tracker.lock().create_goal("goal".into(), "retain silence".into(), None, 0, chrono::Utc::now().to_rfc3339(), None);
            let actor = Arc::new(actor);
            let (session, actor_task) = support::spawn_completion_receiver(actor.clone(), events);
            let (mut config, mut publications, _unused_persistence, _unused_commands) = make_test_config_full_raw();
            config.session_id = info.id.clone();
            config.persistence = persistence.handle.clone();
            config.session_cmd_tx = session.clone();
            config.goal_loop_active.store(true, Ordering::Relaxed);
            let bridge = spawn_notification_bridge(config);
            let terminal = LocalTerminalBackend::new_local(Default::default());
            let background = bounded(terminal.run_background(TerminalRunRequest {
                command: "printf goal-terminal-evidence".into(), working_directory: "/tmp".into(), env: Default::default(),
                timeout: std::time::Duration::from_secs(10), output_byte_limit: 4096,
                output_file: directory.path().join("terminal-output"), notification_handle: bridge,
                tool_call_id: format!("goal-producer-{kind:?}"), display_command: None, auto_background_on_timeout: false,
                foreground_block_budget: None, kind, owner_session_id: Some(info.id.to_string()), description: None,
            })).await.unwrap();
            bounded(async {
                loop {
                    if let Some(xai_acp_lib::AcpClientMessage::ExtNotification(message)) = publications.recv().await
                        && message.request.method.as_ref() == "x.ai/task_completed" { break; }
                }
            }).await;
            let state = storage.read_completion_state(&info).await.unwrap();
            assert!(state.receipts.is_empty());
            let [CompletionObligation::Suppressed { id, reason: crate::session::completion_delivery::CompletionSuppression::GoalNotificationPolicy, .. }] = &state.obligations[..] else { panic!("real goal publication must have a durable policy disposition") };
            let id = id.clone();
            let [CompletionObligation::Suppressed { source: crate::session::completion_delivery::SuppressedCompletionSource::Enrolled(payload), .. }] = &state.obligations[..] else { panic!("suppression must retain enrollment") };
            assert_original_terminal_payload(&id, payload, &background.task_id, kind, "goal-terminal-evidence", &directory.path().join("terminal-output"));
            assert!(actor.state.lock().await.pending_inputs.is_empty());
            assert!(actor.state.lock().await.running_task.is_none());
            assert!(bounded(terminal.get_task(&background.task_id)).await.unwrap().completed);
            assert!(actor.chat_state_handle.snapshot().await.unwrap().conversation.is_empty());
            drop(actor_task);
            drop(actor);
            drop(session);
            drop(terminal);
            drop(persistence_task);
            tokio::task::yield_now().await;
            let reopened = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            let recovered = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
            assert!(matches!(&recovered.obligations[..], [CompletionObligation::Suppressed { id: restored, .. }] if restored == &id));
            let [CompletionObligation::Suppressed { source: crate::session::completion_delivery::SuppressedCompletionSource::Enrolled(payload), .. }] = &recovered.obligations[..] else { panic!("reopen must retain the original snapshot") };
            assert_original_terminal_payload(&id, payload, &background.task_id, kind, "goal-terminal-evidence", &directory.path().join("terminal-output"));
            assert!(recovered.receipts.is_empty());
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), reopened.clone());
            let _persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (gateway, gateway_rx) = mpsc::unbounded_channel();
            support::drain_gateway(gateway_rx);
            let (replacement, events) = support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                gateway, persistence.handle.tx.clone(), Box::new(ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
            let replacement = Arc::new(replacement);
            let (session, _actor_task) = support::spawn_completion_receiver(replacement.clone(), events);
            session.send(SessionCommand::ResumeTerminalCompletion { id, text: "forbidden goal replay".into(), policy: CompletionEventPolicy::Automatic }).unwrap();
            let (reply, response) = tokio::sync::oneshot::channel();
            session.send(SessionCommand::GetToolOverrides { respond_to: reply }).unwrap();
            bounded(response).await.unwrap();
            assert!(replacement.state.lock().await.pending_inputs.is_empty());
            assert!(replacement.state.lock().await.running_task.is_none());
            assert!(replacement.chat_state_handle.snapshot().await.unwrap().conversation.is_empty());
            assert!(reopened.read_completion_state(&info).await.unwrap().receipts.is_empty());
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn actual_producer_enrollment_before_goal_admission_restarts_under_the_common_policy_owner() {
    tokio::task::LocalSet::new().run_until(async {
        for kind in [TaskKind::Bash, TaskKind::Monitor] {
            let directory = tempfile::tempdir().unwrap();
            let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
            let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
            let persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (mut config, mut publications, _unused_persistence, mut admissions) = make_test_config_full_raw();
            config.session_id = info.id.clone();
            config.persistence = persistence.handle.clone();
            config.goal_loop_active.store(true, Ordering::Relaxed);
            let bridge = spawn_notification_bridge(config);
            let terminal = LocalTerminalBackend::new_local(Default::default());
            let background = bounded(terminal.run_background(TerminalRunRequest {
                command: "printf enrollment-window-evidence".into(), working_directory: "/tmp".into(), env: Default::default(),
                timeout: std::time::Duration::from_secs(10), output_byte_limit: 4096,
                output_file: directory.path().join("terminal-output"), notification_handle: bridge,
                tool_call_id: format!("enrollment-window-{kind:?}"), display_command: None, auto_background_on_timeout: false,
                foreground_block_budget: None, kind, owner_session_id: Some(info.id.to_string()), description: None,
            })).await.unwrap();
            let held_policy_ack = bounded(async {
                loop {
                    let command = admissions.recv().await.unwrap();
                    if let SessionCommand::ClassifyTerminalNotification { respond_to, .. } = command { break respond_to; }
                    assert!(!matches!(command, SessionCommand::Prompt { .. } | SessionCommand::InjectNotification { .. }), "producer published before actor policy admission");
                }
            }).await;
            let state = storage.read_completion_state(&info).await.unwrap();
            let [CompletionObligation::Pending { id, payload }] = &state.obligations[..] else { panic!("enrollment must precede actor admission") };
            let id = id.clone();
            let text = payload.model_text(&id);
            assert_original_terminal_payload(&id, payload, &background.task_id, kind, "enrollment-window-evidence", &directory.path().join("terminal-output"));
            assert!(text.contains(&background.task_id));
            assert!(state.receipts.is_empty());
            while let Ok(message) = publications.try_recv() {
                if let xai_acp_lib::AcpClientMessage::ExtNotification(message) = message {
                    assert_ne!(message.request.method.as_ref(), "x.ai/task_completed", "producer cannot publish across the unacknowledged policy boundary");
                }
            }
            drop(terminal);
            drop(persistence_task);
            let reopened = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            let recovered = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
            assert!(matches!(&recovered.obligations[..], [CompletionObligation::Pending { id: restored, .. }] if restored == &id));
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), reopened.clone());
            let _persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (gateway, gateway_rx) = mpsc::unbounded_channel();
            support::drain_gateway(gateway_rx);
            let (replacement, events) = support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                gateway, persistence.handle.tx.clone(), Box::new(ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
            support::set_goal_harness_for_tests(&replacement);
            replacement.goal_tracker.lock().create_goal("goal".into(), "recover policy".into(), None, 0, chrono::Utc::now().to_rfc3339(), None);
            let replacement = Arc::new(replacement);
            let (session, _actor_task) = support::spawn_completion_receiver(replacement.clone(), events);
            session.send(SessionCommand::ResumeTerminalCompletion { id: id.clone(), text, policy: CompletionEventPolicy::Automatic }).unwrap();
            let (reply, response) = tokio::sync::oneshot::channel();
            session.send(SessionCommand::GetToolOverrides { respond_to: reply }).unwrap();
            bounded(response).await.unwrap();
            let state = reopened.read_completion_state(&info).await.unwrap();
            assert!(matches!(&state.obligations[..], [CompletionObligation::Suppressed { id: restored, .. }] if restored == &id));
            assert!(state.receipts.is_empty());
            assert!(replacement.state.lock().await.pending_inputs.is_empty());
            assert!(replacement.state.lock().await.running_task.is_none());
            assert!(replacement.chat_state_handle.snapshot().await.unwrap().conversation.is_empty());
            drop(admissions);
            drop(held_policy_ack);
        }
    }).await;
}
