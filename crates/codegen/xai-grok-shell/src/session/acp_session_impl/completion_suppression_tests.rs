use super::*;
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter};
use crate::session::completion_delivery::{CompletionObligation, CompletionSuppression};

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(10), future).await.expect("suppression barrier timed out")
}

#[tokio::test(flavor = "current_thread")]
async fn batch_refreshes_a_racing_disposition_without_losing_automatic_peers() {
    use xai_grok_tools::computer::types::{TaskKind, TaskSnapshot};
    use xai_chat_state::completion::{CompletionIdentity, CompletionTaskKind, TerminalCompletionId};
    use crate::session::completion_delivery::{CompletionPayloadSource, CompletionEventPolicy};
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        for (id, kind, text) in [("suppressed", TaskKind::Bash, "must stay silent"), ("automatic", TaskKind::Monitor, "automatic peer evidence")] {
            let snapshot = TaskSnapshot { task_id: id.into(), command: "finished".into(), display_command: None, cwd: "/tmp".into(), start_time: std::time::SystemTime::UNIX_EPOCH, end_time: Some(std::time::SystemTime::UNIX_EPOCH), output: text.into(), output_file: Default::default(), truncated: false, exit_code: Some(0), signal: None, completed: true, kind, block_waited: false, explicitly_killed: false, kill_result_delivered: false, owner_session_id: Some(info.id.to_string()), description: None, is_backgrounded: true, output_total_bytes: text.len() };
            storage.enroll_terminal_completion(&info, CompletionPayloadSource::Task(Box::new(snapshot)), text.into(), CompletionEventPolicy::Automatic).await.unwrap();
        }
        let suppressed: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "suppressed".into(), task_kind: CompletionTaskKind::Bash }.into();
        let automatic: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "automatic".into(), task_kind: CompletionTaskKind::Monitor }.into();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
        let _owner = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (proxy, mut requests) = mpsc::unbounded_channel();
        let real = persistence.handle.tx.clone();
        let _proxy = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local({
            let storage = storage.clone();
            let suppressed = suppressed.clone();
            async move {
                let mut inject_race = true;
                while let Some(request) = requests.recv().await {
                    match request {
                        PersistenceMsg::GetCompletionState { respond_to } if inject_race => {
                            let stale = storage.read_completion_state(&info).await.unwrap();
                            storage.suppress_terminal_completions(&info, vec![suppressed.clone()], CompletionSuppression::GoalNotificationPolicy).await.unwrap();
                            inject_race = false;
                            respond_to.send(Ok(stale)).unwrap();
                        }
                        other => { real.send(other).unwrap(); }
                    }
                }
            }
        }));
        let (gateway, gateway_rx) = mpsc::unbounded_channel();
        super::super::super::support::drain_gateway(gateway_rx);
        let (actor, _) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
            gateway, proxy.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(proxy))).await;
        actor.state.lock().await.running_task = Some(super::super::super::support::running_task_stub("actual-caller"));
        assert!(bounded(actor.project_terminal_batch(vec![suppressed, automatic], "actual-caller", "stale batch must not leak".into())).await.unwrap());
        let history = actor.chat_state_handle.snapshot().await.unwrap().conversation;
        assert!(history.iter().all(|item| !item.text_content().contains("must stay silent") && !item.text_content().contains("stale batch must not leak")));
        assert_eq!(history.iter().filter(|item| item.text_content() == "automatic peer evidence").count(), 1);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn actual_notification_actor_commits_suppression_before_reported_cache_and_restores_failed_ack() {
    tokio::task::LocalSet::new().run_until(async {
        #[derive(Clone, Copy)]
        enum SuppressionAck { Success, Error, LostBeforeCommit, LostAfterCommit }
        for outcome in [SuppressionAck::Success, SuppressionAck::Error, SuppressionAck::LostBeforeCommit, SuppressionAck::LostAfterCommit] {
            let fail_ack = !matches!(outcome, SuppressionAck::Success);
            let (gateway, gateway_rx) = mpsc::unbounded_channel();
            super::super::super::support::drain_gateway(gateway_rx);
            let (raw, mut requests) = mpsc::unbounded_channel();
            let (mut actor, events) = super::super::super::support::create_test_actor_ex(0, 256_000, 85, gateway, raw.clone()).await;
            actor.notifications.persistence_tx = raw;
            actor.last_live_orphan_reconcile.set(Some(std::time::Instant::now()));
            actor.goal_harness_enabled.store(true, std::sync::atomic::Ordering::Relaxed);
            actor.goal_tracker.lock().create_goal("goal".into(), "retain silence".into(), None, 0, chrono::Utc::now().to_rfc3339(), None);
            let actor = Arc::new(actor);
            let (sender, _task) = super::super::super::support::spawn_completion_receiver(actor.clone(), events);
            sender.send(SessionCommand::InjectNotification {
                prompt_id: "goal-completion".into(), prompt_blocks: vec![acp::ContentBlock::from("goal-owned terminal")],
                priority: NotificationPriority::Later, source: NotificationSource::BashTaskCompleted { task_id: "suppressed-execution".into() },
            }).unwrap();
            let (ids, reason, ack) = bounded(async {
                loop {
                    if let PersistenceMsg::SuppressTerminalCompletions { ids, reason, respond_to } = requests.recv().await.unwrap() { break (ids, reason, respond_to); }
                }
            }).await;
            let mut reported = true;
            actor.with_reported_completions(|cache| reported = cache.is_reported("suppressed-execution")).await;
            assert!(!reported, "derived reporting cannot precede durable policy disposition");
            assert!(actor.state.lock().await.running_task.is_none());
            if fail_ack { actor.state.lock().await.notifications_suppressed = true; }
            if matches!(outcome, SuppressionAck::Error | SuppressionAck::LostBeforeCommit) {
                match outcome {
                    SuppressionAck::Error => ack.send(Err(std::io::Error::other("injected suppression refusal"))).unwrap(),
                    SuppressionAck::LostBeforeCommit => drop(ack),
                    SuppressionAck::Success | SuppressionAck::LostAfterCommit => unreachable!(),
                }
            } else {
                let directory = tempfile::tempdir().unwrap();
                let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
                storage.init_session(&actor.session_info, acp::ModelId::new("test")).await.unwrap();
                storage.suppress_terminal_completions(&actor.session_info, ids.clone(), reason).await.unwrap();
                let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
                let state = reopened.recover_subagent_completions(&actor.session_info).await.unwrap().completion_delivery;
                assert!(state.receipts.is_empty());
                assert!(matches!(&state.obligations[..], [CompletionObligation::Suppressed { id, reason: CompletionSuppression::GoalNotificationPolicy, .. }] if id == &ids[0]));
                actor.with_reported_completions(|cache| reported = cache.is_reported("suppressed-execution")).await;
                assert!(!reported, "even a committed disposition cannot update the cache until its ACK arrives");
                match outcome {
                    SuppressionAck::Success => ack.send(Ok(())).unwrap(),
                    SuppressionAck::LostAfterCommit => {
                        drop(ack);
                        let (gateway, gateway_rx) = mpsc::unbounded_channel();
                        super::super::super::support::drain_gateway(gateway_rx);
                        let persistence = crate::session::persistence::durable_update_tests::test_actor(actor.session_info.clone(), Arc::new(reopened));
                        let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
                        let (replacement, events) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                            gateway, persistence.handle.tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
                        let replacement = Arc::new(replacement);
                        let (sender, _receiver) = super::super::super::support::spawn_completion_receiver(replacement.clone(), events);
                        sender.send(SessionCommand::ResumeTerminalCompletion { id: ids[0].clone(), text: "lost ACK must not replay".into(), policy: crate::session::completion_delivery::CompletionEventPolicy::Automatic }).unwrap();
                        let (reply, response) = oneshot::channel();
                        sender.send(SessionCommand::GetToolOverrides { respond_to: reply }).unwrap();
                        bounded(response).await.unwrap();
                        assert!(replacement.state.lock().await.pending_inputs.is_empty());
                        assert!(replacement.state.lock().await.running_task.is_none());
                        assert!(replacement.chat_state_handle.snapshot().await.unwrap().conversation.is_empty());
                    }
                    SuppressionAck::Error | SuppressionAck::LostBeforeCommit => unreachable!(),
                }
            }
            let (reply, response) = oneshot::channel();
            sender.send(SessionCommand::GetToolOverrides { respond_to: reply }).unwrap();
            bounded(response).await.unwrap();
            actor.with_reported_completions(|cache| reported = cache.is_reported("suppressed-execution")).await;
            assert_eq!(reported, !fail_ack);
            assert_eq!(actor.state.lock().await.pending_notifications.len(), usize::from(fail_ack));
            assert!(actor.state.lock().await.running_task.is_none());
            assert!(actor.chat_state_handle.snapshot().await.unwrap().conversation.iter().all(|item| !item.text_content().contains("goal-owned terminal")));
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn real_goal_producer_retains_suppression_errors_and_recovers_postcommit_lost_ack() {
    use crate::tools::notification_bridge::{spawn_notification_bridge, tests::make_test_config_full_raw};
    use xai_grok_tools::computer::local::terminal::LocalTerminalBackend;
    use xai_grok_tools::computer::types::{TaskKind, TerminalBackend, TerminalRunRequest};
    use crate::session::completion_delivery::CompletionEventPolicy;

    #[derive(Clone, Copy)]
    enum Failure { Refused, CommittedAckLost }

    tokio::task::LocalSet::new().run_until(async {
        for kind in [TaskKind::Bash, TaskKind::Monitor] {
            for failure in [Failure::Refused, Failure::CommittedAckLost] {
                let directory = tempfile::tempdir().unwrap();
                let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
                let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
                storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
                let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
                let persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
                let (proxy, mut requests) = mpsc::unbounded_channel();
                let (held, mut suppressions) = mpsc::unbounded_channel();
                let real = persistence.handle.tx.clone();
                let proxy_task = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(async move {
                    while let Some(request) = requests.recv().await {
                        match request {
                            PersistenceMsg::SuppressTerminalCompletions { ids, reason, respond_to } => {
                                held.send((ids, reason, respond_to)).unwrap();
                            }
                            other => { real.send(other).unwrap(); }
                        }
                    }
                }));
                let (gateway, gateway_rx) = mpsc::unbounded_channel();
                super::super::super::support::drain_gateway(gateway_rx);
                let (actor, events) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                    gateway, proxy.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(proxy.clone()))).await;
                super::super::super::support::set_goal_harness_for_tests(&actor);
                actor.goal_tracker.lock().create_goal("goal".into(), "retain producer silence".into(), None, 0, chrono::Utc::now().to_rfc3339(), None);
                let actor = Arc::new(actor);
                let (session, receiver) = super::super::super::support::spawn_completion_receiver(actor.clone(), events);
                let (mut config, mut publications, _unused_persistence, _unused_commands) = make_test_config_full_raw();
                config.session_id = info.id.clone();
                config.persistence = crate::session::persistence::PersistenceHandle::from_sender_for_test(proxy);
                config.session_cmd_tx = session.clone();
                config.goal_loop_active.store(true, std::sync::atomic::Ordering::Relaxed);
                let terminal = LocalTerminalBackend::new_local(Default::default());
                let background = bounded(terminal.run_background(TerminalRunRequest {
                    command: "printf suppression-failure-evidence".into(), working_directory: "/tmp".into(), env: Default::default(),
                    timeout: std::time::Duration::from_secs(10), output_byte_limit: 4096,
                    output_file: directory.path().join("terminal-output"), notification_handle: spawn_notification_bridge(config),
                    tool_call_id: format!("failed-policy-{kind:?}"), display_command: None, auto_background_on_timeout: false,
                    foreground_block_budget: None, kind, owner_session_id: Some(info.id.to_string()), description: None,
                })).await.unwrap();
                let (ids, reason, ack) = bounded(suppressions.recv()).await.unwrap();
                let pending = storage.read_completion_state(&info).await.unwrap();
                let [CompletionObligation::Pending { id, payload }] = &pending.obligations[..] else { panic!("producer must remain enrolled before suppression ACK") };
                assert_eq!(ids, vec![id.clone()]);
                let id = id.clone();
                let text = payload.model_text(&id);
                let mut reported = true;
                actor.with_reported_completions(|cache| reported = cache.is_reported(&background.task_id)).await;
                assert!(!reported);
                assert!(actor.state.lock().await.pending_inputs.is_empty());
                assert!(actor.state.lock().await.running_task.is_none());
                assert!(actor.chat_state_handle.snapshot().await.unwrap().conversation.is_empty());
                actor.state.lock().await.notifications_suppressed = true;
                match failure {
                    Failure::Refused => ack.send(Err(std::io::Error::other("injected policy refusal"))).unwrap(),
                    Failure::CommittedAckLost => {
                        let (respond_to, response) = oneshot::channel();
                        persistence.handle.tx.send(PersistenceMsg::SuppressTerminalCompletions { ids, reason, respond_to }).unwrap();
                        bounded(response).await.unwrap().unwrap();
                        let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
                        assert!(matches!(&reopened.read_completion_state(&info).await.unwrap().obligations[..], [CompletionObligation::Suppressed { id: restored, .. }] if restored == &id));
                        actor.with_reported_completions(|cache| reported = cache.is_reported(&background.task_id)).await;
                        assert!(!reported, "the production storage ACK has not reached the notification actor");
                        drop(ack);
                    }
                }
                let (reply, response) = oneshot::channel();
                session.send(SessionCommand::GetToolOverrides { respond_to: reply }).unwrap();
                bounded(response).await.unwrap();
                actor.with_reported_completions(|cache| reported = cache.is_reported(&background.task_id)).await;
                assert!(!reported);
                assert!(actor.state.lock().await.pending_notifications.iter().any(|row| row.source.task_id() == background.task_id.as_str()));
                assert!(actor.state.lock().await.pending_inputs.is_empty());
                assert!(actor.state.lock().await.running_task.is_none());
                assert!(actor.chat_state_handle.snapshot().await.unwrap().conversation.is_empty());
                while let Ok(message) = publications.try_recv() {
                    if let xai_acp_lib::AcpClientMessage::ExtNotification(message) = message {
                        assert_ne!(message.request.method.as_ref(), "x.ai/task_completed", "failed policy ACK must not release producer publication");
                    }
                }
                drop(receiver);
                drop(actor);
                drop(session);
                drop(terminal);
                drop(proxy_task);
                drop(persistence_task);
                tokio::task::yield_now().await;
                let reopened = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
                let recovered = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
                assert!(recovered.receipts.is_empty());
                match failure {
                    Failure::Refused => assert!(matches!(&recovered.obligations[..], [CompletionObligation::Pending { id: restored, .. }] if restored == &id)),
                    Failure::CommittedAckLost => assert!(matches!(&recovered.obligations[..], [CompletionObligation::Suppressed { id: restored, .. }] if restored == &id)),
                }
                let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), reopened.clone());
                let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
                let (gateway, gateway_rx) = mpsc::unbounded_channel();
                super::super::super::support::drain_gateway(gateway_rx);
                let (replacement, events) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                    gateway, persistence.handle.tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
                super::super::super::support::set_goal_harness_for_tests(&replacement);
                replacement.goal_tracker.lock().create_goal("goal".into(), "recover producer silence".into(), None, 0, chrono::Utc::now().to_rfc3339(), None);
                let replacement = Arc::new(replacement);
                let (session, _receiver) = super::super::super::support::spawn_completion_receiver(replacement.clone(), events);
                session.send(SessionCommand::ResumeTerminalCompletion { id: id.clone(), text, policy: CompletionEventPolicy::Automatic }).unwrap();
                let (reply, response) = oneshot::channel();
                session.send(SessionCommand::GetToolOverrides { respond_to: reply }).unwrap();
                bounded(response).await.unwrap();
                assert!(replacement.state.lock().await.pending_inputs.is_empty());
                assert!(replacement.state.lock().await.running_task.is_none());
                assert!(replacement.chat_state_handle.snapshot().await.unwrap().conversation.is_empty());
                let recovered = reopened.read_completion_state(&info).await.unwrap();
                assert!(recovered.receipts.is_empty());
                assert!(matches!(&recovered.obligations[..], [CompletionObligation::Suppressed { id: restored, .. }] if restored == &id));
            }
        }
    }).await;
}
