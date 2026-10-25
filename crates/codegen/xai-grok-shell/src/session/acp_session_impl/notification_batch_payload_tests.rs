use super::*;
use crate::session::completion_delivery::{CompletionEventPolicy, CompletionPayloadSource, CompletionSuppression};
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter};
use xai_chat_state::completion::{CompletionIdentity, CompletionTaskKind, TerminalCompletionId};
use xai_grok_tools::computer::types::{TaskKind, TaskSnapshot};

fn terminal(info: &crate::session::SessionInfo, task_id: &str, kind: TaskKind, text: &str) -> TaskSnapshot {
    TaskSnapshot {
        task_id: task_id.into(), command: "finished".into(), display_command: None, cwd: "/tmp".into(),
        start_time: std::time::SystemTime::UNIX_EPOCH, end_time: Some(std::time::SystemTime::UNIX_EPOCH),
        output: text.into(), output_file: Default::default(), truncated: false, exit_code: Some(0), signal: None,
        completed: true, kind, block_waited: false, explicitly_killed: false, kill_result_delivered: false,
        owner_session_id: Some(info.id.to_string()), description: None, is_backgrounded: true, output_total_bytes: text.len(),
    }
}

fn row(task_id: &str, text: &str, source: NotificationSource) -> PendingNotification {
    PendingNotification { prompt_id: task_id.into(), prompt_blocks: vec![acp::ContentBlock::from(text)], priority: NotificationPriority::Later, source }
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(10), future).await.expect("notification batch rendezvous timed out")
}

#[tokio::test(flavor = "current_thread")]
async fn actual_drain_projection_preserves_unrelated_observations_with_exact_terminal_receipts() {
    tokio::task::LocalSet::new().run_until(async {
        for suppress in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
            let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
            let eligible: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "eligible-monitor".into(), task_kind: CompletionTaskKind::Monitor }.into();
            storage.enroll_terminal_completion(&info,
                CompletionPayloadSource::Task(Box::new(terminal(&info, "eligible-monitor", TaskKind::Monitor, "eligible terminal evidence"))),
                "eligible terminal evidence".into(), CompletionEventPolicy::Automatic).await.unwrap();
            let suppressed: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "silent-bash".into(), task_kind: CompletionTaskKind::Bash }.into();
            if suppress {
                storage.enroll_terminal_completion(&info,
                    CompletionPayloadSource::Task(Box::new(terminal(&info, "silent-bash", TaskKind::Bash, "forbidden terminal evidence"))),
                    "forbidden terminal evidence".into(), CompletionEventPolicy::Automatic).await.unwrap();
                storage.suppress_terminal_completions(&info, vec![suppressed.clone()], CompletionSuppression::GoalNotificationPolicy).await.unwrap();
            }
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
            let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (gateway, gateway_rx) = mpsc::unbounded_channel();
            super::super::super::support::drain_gateway(gateway_rx);
            let (actor, _) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                gateway, persistence.handle.tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
            let mut rows = vec![
                row("eligible-monitor", "eligible terminal evidence", NotificationSource::MonitorCompleted { task_id: "eligible-monitor".into() }),
                row("independent-monitor", "independent transient evidence", NotificationSource::MonitorEvent { task_id: "independent-monitor".into() }),
                row("eligible-monitor", "superseded transient evidence", NotificationSource::MonitorEvent { task_id: "eligible-monitor".into() }),
            ];
            if suppress {
                rows.insert(0, row("silent-bash", "forbidden terminal evidence", NotificationSource::BashTaskCompleted { task_id: "silent-bash".into() }));
            }
            let input = {
                let mut state = actor.state.lock().await;
                assert!(actor.queue_notification_batch(&mut state, rows));
                state.pending_inputs.pop_front().unwrap()
            };
            actor.state.lock().await.running_task = Some(super::super::super::support::running_task_stub("batch-caller"));
            let text = SessionActor::queue_text_from_blocks(&input.prompt_blocks);
            assert_eq!(bounded(actor.project_terminal_input(&input.input_origin, "batch-caller", text.clone())).await.unwrap(), Some(true));
            assert_eq!(bounded(actor.project_terminal_input(&input.input_origin, "batch-caller", text)).await.unwrap(), Some(false));
            let history = actor.chat_state_handle.snapshot().await.unwrap().conversation;
            assert_eq!(history.iter().filter(|item| item.text_content().contains("eligible terminal evidence")).count(), 1);
            assert_eq!(history.iter().filter(|item| item.text_content().contains("independent transient evidence")).count(), 1);
            assert!(history.iter().all(|item| !item.text_content().contains("forbidden terminal evidence") && !item.text_content().contains("superseded transient evidence")));
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].text_content().matches("eligible terminal evidence").count(), 1);
            assert_eq!(history[0].text_content().matches("independent transient evidence").count(), 1);
            let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
            let recovered = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
            assert_eq!(recovered.receipts.iter().map(|receipt| receipt.completion.clone()).collect::<Vec<_>>(), vec![eligible]);
            assert!(recovered.receipts.iter().all(|receipt| receipt.completion != suppressed));
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn bridge_enrollment_preserves_selected_wait_then_projects_an_ordinary_automatic_peer() {
    use crate::tools::notification_bridge::{spawn_notification_bridge, tests::make_test_config_full_raw};
    use crate::session::completion_delivery::CompletionObligation;
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
        let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (gateway, gateway_rx) = mpsc::unbounded_channel();
        super::super::super::support::drain_gateway(gateway_rx);
        let (actor, events) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
            gateway, persistence.handle.tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
        actor.goal_harness_enabled.store(true, std::sync::atomic::Ordering::Relaxed);
        actor.goal_tracker.lock().create_goal("goal".into(), "selected wait".into(), None, 0, chrono::Utc::now().to_rfc3339(), None);
        actor.record_turn_wait(TurnWaitDeclaration::of([WaitOnTarget::Monitor { task_id: "selected-monitor".into() }]));
        actor.state.lock().await.running_task = Some(super::super::super::support::running_task_stub("selected-caller"));
        let actor = Arc::new(actor);
        let (session, _receiver) = super::super::super::support::spawn_completion_receiver(actor.clone(), events);
        let (mut config, publications, _unused_persistence, _unused_commands) = make_test_config_full_raw();
        super::super::super::support::drain_gateway(publications);
        config.session_id = info.id.clone();
        config.persistence = persistence.handle.clone();
        config.session_cmd_tx = session.clone();
        config.goal_loop_active.store(true, std::sync::atomic::Ordering::Relaxed);
        let goal_flag = config.goal_loop_active.clone();
        let bridge = spawn_notification_bridge(config);
        let selected = terminal(&info, "selected-monitor", TaskKind::Monitor, "selected outcome");
        bounded(bridge.enroll_task_completion(selected.clone()).wait()).await.unwrap();
        assert!(!actor.turn_wait_declaration.borrow().declaration.is_empty(), "enrollment policy observation must not consume a declared wait");
        let ignored = terminal(&info, "goal-peer", TaskKind::Bash, "ignored goal outcome");
        bounded(bridge.enroll_task_completion(ignored.clone()).wait()).await.unwrap();
        assert!(storage.read_completion_state(&info).await.unwrap().obligations.iter().any(|obligation| matches!(obligation, CompletionObligation::Suppressed { id, .. } if matches!(id, CompletionIdentity::V2 { terminal: TerminalCompletionId::Task { task_id, .. }, .. } if task_id == "goal-peer"))));
        bridge.send_task_complete(ignored);
        bridge.send_task_complete(selected);
        bounded(async {
            loop {
                if actor.state.lock().await.pending_inputs.iter().any(|input| matches!(input.input_origin.as_prompt_origin(), PromptOrigin::TaskCompleted { task_id } if task_id == "selected-monitor")) { break; }
                tokio::task::yield_now().await;
            }
        }).await;
        assert!(actor.turn_wait_declaration.borrow().declaration.is_empty());
        actor.goal_harness_enabled.store(false, std::sync::atomic::Ordering::Relaxed);
        goal_flag.store(false, std::sync::atomic::Ordering::Relaxed);
        let automatic = terminal(&info, "ordinary-automatic", TaskKind::Bash, "ordinary automatic outcome");
        bounded(bridge.enroll_task_completion(automatic.clone()).wait()).await.unwrap();
        bridge.send_task_complete(automatic);
        bounded(async {
            loop {
                if actor.state.lock().await.pending_inputs.iter().any(|input| matches!(input.input_origin.as_prompt_origin(), PromptOrigin::TaskCompleted { task_id } if task_id == "ordinary-automatic")) { break; }
                tokio::task::yield_now().await;
            }
        }).await;
        assert!(bounded(actor.drain_terminal_wakes_at_safe_point()).await);
        assert!(!bounded(actor.drain_terminal_wakes_at_safe_point()).await);
        let history = actor.chat_state_handle.snapshot().await.unwrap().conversation;
        assert_eq!(history.iter().filter(|item| item.text_content().contains("selected-monitor")).count(), 1);
        assert_eq!(history.iter().filter(|item| item.text_content().contains("ordinary-automatic")).count(), 1);
        assert!(history.iter().all(|item| !item.text_content().contains("goal-peer")));
        let state = storage.read_completion_state(&info).await.unwrap();
        assert_eq!(state.receipts.len(), 2);
        assert!(state.receipts.iter().all(|receipt| matches!(&receipt.completion, CompletionIdentity::V2 { terminal: TerminalCompletionId::Task { task_id, .. }, .. } if task_id == "selected-monitor" || task_id == "ordinary-automatic")));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn preexisting_tool_receipt_does_not_consume_a_new_unrelated_observation() {
    use crate::session::completion_delivery::CompletionObligation;
    tokio::task::LocalSet::new().run_until(async {
        for settled in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
            let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
            let id: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: "test-actor".into(), task_id: "observed-monitor".into(), task_kind: CompletionTaskKind::Monitor }.into();
            storage.enroll_terminal_completion(&info, CompletionPayloadSource::Task(Box::new(terminal(&info, "observed-monitor", TaskKind::Monitor, "original snapshot output"))),
                "original enrolled terminal text".into(), CompletionEventPolicy::Automatic).await.unwrap();
            let admission = xai_chat_state::InputAdmission { parent_task: xai_chat_state::ParentTask::Instance { id: "original-observer-instance".into() }, cancellation: tokio_util::sync::CancellationToken::new() };
            let mut observed = xai_chat_state::CompletionProjection::new(id.clone(), "original-observer".into(), "unused".into());
            observed.item = ConversationItem::tool_result("actual-terminal-read", "previously observed terminal payload");
            observed.receipt.parent_task = admission.parent_task.clone();
            assert!(matches!(storage.append_completion_commit_aware(&info, &observed, &admission).await.unwrap(), xai_chat_state::CompletionProjectionAck::Appended));
            let old_history = match storage.append_completion_commit_aware(&info, &observed, &admission).await.unwrap() {
                xai_chat_state::CompletionProjectionAck::AlreadyPresent { history } => history,
                other => panic!("expected existing observed history: {other:?}"),
            };
            assert_eq!(old_history.len(), 1);
            assert_eq!(old_history[0].text_content(), "previously observed terminal payload");
            assert!(!old_history[0].text_content().contains("new unrelated observation"));
            if settled {
                storage.append_update_durable_commit_aware(&info, &crate::session::storage::SessionUpdate::Xai(Box::new(crate::extensions::notification::SessionNotification {
                    session_id: info.id.clone(), meta: Some(serde_json::json!({ "_grokParentTask": admission.parent_task, "_receiverTurnOrigin": crate::session::delivery_receiver::ReceiverTurnOrigin::Other })),
                    update: crate::extensions::notification::SessionUpdate::TurnCompleted { prompt_id: "original-observer".into(), stop_reason: "end_turn".into(), agent_result: None, error_kind: None, usage: None, elapsed_ms: None },
                }))).await.unwrap();
            }
            let before = storage.recover_subagent_completions(&info).await.unwrap().completion_delivery;
            if settled { assert!(before.obligations.is_empty()); }
            else {
                assert!(matches!(&before.obligations[..], [CompletionObligation::Projected { receipt, payload }] if receipt.completion == id && payload.text == "original enrolled terminal text"));
            }
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
            let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (gateway, gateway_rx) = mpsc::unbounded_channel();
            super::super::super::support::drain_gateway(gateway_rx);
            let (actor, _) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                gateway, persistence.handle.tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
            assert!(matches!(actor.chat_state_handle.append_completion_and_ack(observed, admission).await.unwrap(), xai_chat_state::CompletionProjectionAck::AlreadyPresent { .. }));
            let input = {
                let mut state = actor.state.lock().await;
                state.running_task = Some(super::super::super::support::running_task_stub("new-observation-caller"));
                assert!(actor.queue_notification_batch(&mut state, vec![
                    row("observed-monitor", "terminal notification must not duplicate", NotificationSource::MonitorCompleted { task_id: "observed-monitor".into() }),
                    row("unrelated-monitor", "new unrelated observation", NotificationSource::MonitorEvent { task_id: "unrelated-monitor".into() }),
                    row("observed-monitor", "superseded own observation", NotificationSource::MonitorEvent { task_id: "observed-monitor".into() }),
                ]));
                state.pending_inputs.pop_front().unwrap()
            };
            let text = SessionActor::queue_text_from_blocks(&input.prompt_blocks);
            assert_eq!(bounded(actor.project_terminal_input(&input.input_origin, "new-observation-caller", text.clone())).await.unwrap(), Some(true));
            assert_eq!(bounded(actor.project_terminal_input(&input.input_origin, "new-observation-caller", text)).await.unwrap(), Some(false));
            let history = actor.chat_state_handle.snapshot().await.unwrap().conversation;
            let text = history.iter().map(ConversationItem::text_content).collect::<Vec<_>>().join("\n");
            assert_eq!(text.matches("previously observed terminal payload").count(), 1);
            assert_eq!(text.matches("new unrelated observation").count(), 1);
            assert!(!text.contains("terminal notification must not duplicate"));
            assert!(!text.contains("superseded own observation"));
            assert_eq!(history.len(), 2);
            let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
            let after = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
            assert_eq!(after.receipts.iter().map(|receipt| receipt.completion.clone()).collect::<Vec<_>>(), vec![id]);
            if !settled {
                let [CompletionObligation::Projected { payload, .. }] = &after.obligations[..] else { panic!("unsettled observation must retain its original payload") };
                assert_eq!(payload.text, "original enrolled terminal text");
                let CompletionPayloadSource::Task(snapshot) = &payload.source else { panic!("original task source must survive the unrelated batch") };
                assert_eq!(snapshot.owner_session_id.as_deref(), Some("test-actor"));
                assert_eq!(snapshot.task_id, "observed-monitor");
                assert_eq!(snapshot.kind, TaskKind::Monitor);
                assert_eq!(snapshot.command, "finished");
                assert_eq!(snapshot.output, "original snapshot output");
                assert_eq!(snapshot.exit_code, Some(0));
            }
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn event_only_and_all_suppressed_batches_have_their_own_acknowledged_identity() {
    tokio::task::LocalSet::new().run_until(async {
        for suppress in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
            let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
            storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
            let mut rows = vec![row("independent", "event-only literal evidence", NotificationSource::MonitorEvent { task_id: "independent".into() })];
            if suppress {
                let id: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: "test-actor".into(), task_id: "silent-monitor".into(), task_kind: CompletionTaskKind::Monitor }.into();
                storage.enroll_terminal_completion(&info, CompletionPayloadSource::Task(Box::new(terminal(&info, "silent-monitor", TaskKind::Monitor, "silent original output"))), "silent original text".into(), CompletionEventPolicy::Automatic).await.unwrap();
                storage.suppress_terminal_completions(&info, vec![id], CompletionSuppression::GoalNotificationPolicy).await.unwrap();
                rows.extend([
                    row("silent-monitor", "forbidden silent original text", NotificationSource::MonitorCompleted { task_id: "silent-monitor".into() }),
                    row("silent-monitor", "forbidden own observation", NotificationSource::MonitorEvent { task_id: "silent-monitor".into() }),
                ]);
            }
            let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
            let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (gateway, gateway_rx) = mpsc::unbounded_channel();
            super::super::super::support::drain_gateway(gateway_rx);
            let (actor, _) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
                gateway, persistence.handle.tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
            let input = {
                let mut state = actor.state.lock().await;
                state.running_task = Some(super::super::super::support::running_task_stub("event-only-caller"));
                assert!(actor.queue_notification_batch(&mut state, rows));
                state.pending_inputs.pop_front().unwrap()
            };
            let text = SessionActor::queue_text_from_blocks(&input.prompt_blocks);
            assert_eq!(bounded(actor.project_terminal_input(&input.input_origin, "event-only-caller", text.clone())).await.unwrap(), Some(true));
            assert_eq!(bounded(actor.project_terminal_input(&input.input_origin, "event-only-caller", text)).await.unwrap(), Some(false));
            let history = actor.chat_state_handle.snapshot().await.unwrap().conversation;
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].text_content().matches("event-only literal evidence").count(), 1);
            assert!(!history[0].text_content().contains("forbidden"));
            let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
            assert!(reopened.read_completion_state(&info).await.unwrap().receipts.is_empty());
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn mixed_batch_retains_ownership_on_error_then_refreshes_a_real_storage_refusal() {
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        for (task_id, kind, text) in [("racing-bash", TaskKind::Bash, "racing original output"), ("eligible-peer", TaskKind::Monitor, "eligible peer literal")] {
            storage.enroll_terminal_completion(&info, CompletionPayloadSource::Task(Box::new(terminal(&info, task_id, kind, text))), text.into(), CompletionEventPolicy::Automatic).await.unwrap();
        }
        let suppressed: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: "test-actor".into(), task_id: "racing-bash".into(), task_kind: CompletionTaskKind::Bash }.into();
        let eligible: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: "test-actor".into(), task_id: "eligible-peer".into(), task_kind: CompletionTaskKind::Monitor }.into();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
        let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (proxy, mut requests) = mpsc::unbounded_channel();
        let (refusals, mut refused) = mpsc::unbounded_channel();
        let real = persistence.handle.tx.clone();
        let _proxy = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local({
            let storage = storage.clone();
            let info = info.clone();
            let suppressed = suppressed.clone();
            async move {
                let mut refuse_transport = true;
                let mut inject_race = true;
                while let Some(request) = requests.recv().await {
                    match request {
                        PersistenceMsg::GetCompletionState { respond_to } if !refuse_transport && inject_race => {
                            let stale = storage.read_completion_state(&info).await.unwrap();
                            storage.suppress_terminal_completions(&info, vec![suppressed.clone()], CompletionSuppression::GoalNotificationPolicy).await.unwrap();
                            inject_race = false;
                            respond_to.send(Ok(stale)).unwrap();
                        }
                        PersistenceMsg::AppendInputAndAck { projection: xai_chat_state::InputProjection::Notification(_), respond_to, .. } if refuse_transport => {
                            refuse_transport = false;
                            respond_to.send(Err(xai_chat_state::StrictAppendError::NotCommitted(std::io::Error::other("injected preappend refusal")))).unwrap();
                        }
                        PersistenceMsg::AppendInputAndAck { projection: projection @ xai_chat_state::InputProjection::Notification(_), admission, respond_to } => {
                            let (reply, response) = oneshot::channel();
                            real.send(PersistenceMsg::AppendInputAndAck { projection, admission, respond_to: reply }).unwrap();
                            let result = response.await.unwrap();
                            if matches!(&result, Err(xai_chat_state::StrictAppendError::NotCommitted(error)) if error.kind() == std::io::ErrorKind::PermissionDenied) {
                                refusals.send(()).unwrap();
                            }
                            respond_to.send(result).unwrap();
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
        {
            let mut state = actor.state.lock().await;
            state.running_task = Some(super::super::super::support::running_task_stub("racing-caller"));
            assert!(actor.queue_notification_batch(&mut state, vec![
                row("racing-bash", "racing original output", NotificationSource::BashTaskCompleted { task_id: "racing-bash".into() }),
                row("eligible-peer", "eligible peer literal", NotificationSource::MonitorCompleted { task_id: "eligible-peer".into() }),
                row("independent", "racing transient literal", NotificationSource::MonitorEvent { task_id: "independent".into() }),
                row("eligible-peer", "own superseded racing event", NotificationSource::MonitorEvent { task_id: "eligible-peer".into() }),
            ]));
        }
        assert!(!bounded(actor.drain_terminal_wakes_at_safe_point()).await);
        assert_eq!(actor.state.lock().await.pending_inputs.len(), 1);
        assert!(actor.chat_state_handle.snapshot().await.unwrap().conversation.is_empty());
        assert!(bounded(actor.drain_terminal_wakes_at_safe_point()).await);
        bounded(refused.recv()).await.unwrap();
        assert!(actor.state.lock().await.pending_inputs.is_empty());
        assert!(!bounded(actor.drain_terminal_wakes_at_safe_point()).await);
        let history = actor.chat_state_handle.snapshot().await.unwrap().conversation;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].text_content().matches("eligible peer literal").count(), 1);
        assert_eq!(history[0].text_content().matches("racing transient literal").count(), 1);
        assert!(!history[0].text_content().contains("racing original output"));
        assert!(!history[0].text_content().contains("own superseded racing event"));
        let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
        let recovered = reopened.read_completion_state(&info).await.unwrap();
        assert_eq!(recovered.receipts.iter().map(|receipt| receipt.completion.clone()).collect::<Vec<_>>(), vec![eligible]);
        assert!(matches!(&recovered.obligations.iter().find(|obligation| obligation.id() == suppressed).unwrap(), crate::session::completion_delivery::CompletionObligation::Suppressed { source: crate::session::completion_delivery::SuppressedCompletionSource::Enrolled(payload), .. } if payload.text == "racing original output"));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn committed_batch_lost_ack_reopens_once_and_finishes_the_replacement_caller() {
    use crate::session::completion_delivery::CompletionObligation;
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let id: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: "test-actor".into(), task_id: "lost-ack-monitor".into(), task_kind: CompletionTaskKind::Monitor }.into();
        storage.enroll_terminal_completion(&info, CompletionPayloadSource::Task(Box::new(terminal(&info, "lost-ack-monitor", TaskKind::Monitor, "original lost-ack snapshot output"))),
            "original lost-ack terminal text".into(), CompletionEventPolicy::Automatic).await.unwrap();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
        let persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (proxy, mut requests) = mpsc::unbounded_channel();
        let (held, mut commits) = mpsc::unbounded_channel();
        let real = persistence.handle.tx.clone();
        let proxy_task = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(async move {
            let mut hold_first = true;
            while let Some(request) = requests.recv().await {
                match request {
                    PersistenceMsg::AppendInputAndAck { projection: projection @ xai_chat_state::InputProjection::Notification(_), admission, respond_to } if hold_first => {
                        hold_first = false;
                        let (reply, response) = oneshot::channel();
                        real.send(PersistenceMsg::AppendInputAndAck { projection, admission, respond_to: reply }).unwrap();
                        assert!(matches!(response.await.unwrap().unwrap(), xai_chat_state::CompletionProjectionAck::Appended));
                        held.send(respond_to).unwrap();
                    }
                    other => { real.send(other).unwrap(); }
                }
            }
        }));
        let (gateway, gateway_rx) = mpsc::unbounded_channel();
        super::super::super::support::drain_gateway(gateway_rx);
        let (actor, _) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
            gateway, proxy.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(proxy))).await;
        let (original_origin, original_text) = {
            let mut state = actor.state.lock().await;
            state.running_task = Some(super::super::super::support::running_task_stub("old-batch-caller"));
            assert!(actor.queue_notification_batch(&mut state, vec![
                row("lost-ack-monitor", "original lost-ack terminal text", NotificationSource::MonitorCompleted { task_id: "lost-ack-monitor".into() }),
                row("independent-monitor", "lost-ack unrelated observation", NotificationSource::MonitorEvent { task_id: "independent-monitor".into() }),
            ]));
            let input = state.pending_inputs.back().unwrap();
            (input.input_origin.clone(), SessionActor::queue_text_from_blocks(&input.prompt_blocks))
        };
        let actor = Arc::new(actor);
        let projection = tokio::task::spawn_local({ let actor = actor.clone(); async move { actor.drain_terminal_wakes_at_safe_point().await } });
        let lost_ack = bounded(commits.recv()).await.unwrap();
        assert_eq!(actor.state.lock().await.pending_inputs.len(), 1);
        let mut reported = true;
        actor.with_reported_completions(|cache| reported = cache.is_reported("lost-ack-monitor")).await;
        assert!(!reported);
        let reopened = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
        let recovered = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
        assert_eq!(recovered.receipts.iter().map(|receipt| receipt.completion.clone()).collect::<Vec<_>>(), vec![id.clone()]);
        assert!(matches!(&recovered.obligations[..], [CompletionObligation::Projected { receipt, payload }] if receipt.completion == id && payload.text == "original lost-ack terminal text"));
        drop(lost_ack);
        assert!(!bounded(projection).await.unwrap());
        assert_eq!(actor.state.lock().await.pending_inputs.len(), 1);
        drop(actor);
        drop(proxy_task);
        drop(persistence_task);
        tokio::task::yield_now().await;
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), reopened.clone());
        let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (terminal_proxy, mut requests) = mpsc::unbounded_channel();
        let (held, mut terminals) = mpsc::unbounded_channel();
        let real = persistence.handle.tx.clone();
        let _proxy = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(async move {
            while let Some(request) = requests.recv().await {
                match request {
                    PersistenceMsg::AppendUpdateDurablyAndAck { update, respond_to } => {
                        let (reply, response) = oneshot::channel();
                        real.send(PersistenceMsg::AppendUpdateDurablyAndAck { update, respond_to: reply }).unwrap();
                        let result = response.await.unwrap();
                        assert!(result.is_ok());
                        held.send((respond_to, result)).unwrap();
                    }
                    other => { real.send(other).unwrap(); }
                }
            }
        }));
        let (gateway, gateway_rx) = mpsc::unbounded_channel();
        super::super::super::support::drain_gateway(gateway_rx);
        let (replacement, events) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
            gateway, terminal_proxy.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(terminal_proxy))).await;
        let (input, mut response) = super::super::super::support::input_with_origin_rx("replacement-batch-caller", PromptOrigin::User);
        let task = super::super::super::support::running_task_stub("replacement-batch-caller");
        let epoch = task.epoch;
        let identity = task.identity.clone();
        let expected_parent = xai_chat_state::ParentTask::Instance { id: identity.as_ref().clone() };
        {
            let mut state = replacement.state.lock().await;
            state.running_task = Some(task);
            state.pending_inputs.push_back(input);
        }
        let replacement = Arc::new(replacement);
        let (session, receiver) = super::super::super::support::spawn_completion_receiver(replacement.clone(), events);
        session.send(SessionCommand::ResumeTerminalCompletion { id: id.clone(), text: "original lost-ack terminal text".into(), policy: CompletionEventPolicy::Automatic }).unwrap();
        let (reply, fence) = oneshot::channel();
        session.send(SessionCommand::GetToolOverrides { respond_to: reply }).unwrap();
        bounded(fence).await.unwrap();
        assert!(bounded(replacement.drain_terminal_wakes_at_safe_point()).await);
        assert_eq!(bounded(replacement.project_terminal_input(&original_origin, "replacement-batch-caller", original_text)).await.unwrap(), Some(false));
        let history = replacement.chat_state_handle.snapshot().await.unwrap().conversation;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].text_content().matches("original lost-ack terminal text").count(), 1);
        assert_eq!(history[0].text_content().matches("lost-ack unrelated observation").count(), 1);
        let before_terminal = reopened.read_completion_state(&info).await.unwrap();
        assert_eq!(before_terminal.receipts.len(), 1);
        assert!(matches!(&before_terminal.obligations[..], [CompletionObligation::Projected { receipt, .. }] if receipt.parent_task == expected_parent));
        drop(receiver);
        let completion = replacement.handle_completion("replacement-batch-caller".into(), epoch, &identity, ok_end_turn(0, None), None);
        let release = async {
            let (ack, result) = bounded(terminals.recv()).await.unwrap();
            assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)), "caller success must await actual terminal ACK");
            ack.send(result).unwrap();
        };
        let (owned, ()) = tokio::join!(completion, release);
        assert!(owned);
        assert!(bounded(response).await.unwrap().is_ok());
        let settled = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
        assert!(settled.obligations.is_empty());
        assert_eq!(settled.receipts.len(), 1);
        assert_eq!(settled.receipts[0].completion, id);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn deferred_completion_consumption_retains_unrelated_transient_ownership() {
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let eligible: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "deferred-monitor".into(), task_kind: CompletionTaskKind::Monitor }.into();
        storage.enroll_terminal_completion(&info,
            CompletionPayloadSource::Task(Box::new(terminal(&info, "deferred-monitor", TaskKind::Monitor, "deferred terminal evidence"))),
            "deferred terminal evidence".into(), CompletionEventPolicy::Automatic).await.unwrap();
        let silent: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "silent-deferred".into(), task_kind: CompletionTaskKind::Monitor }.into();
        storage.enroll_terminal_completion(&info,
            CompletionPayloadSource::Task(Box::new(terminal(&info, "silent-deferred", TaskKind::Monitor, "forbidden deferred terminal"))),
            "forbidden deferred terminal".into(), CompletionEventPolicy::Automatic).await.unwrap();
        storage.suppress_terminal_completions(&info, vec![silent], CompletionSuppression::GoalNotificationPolicy).await.unwrap();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
        let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (gateway, gateway_rx) = mpsc::unbounded_channel();
        super::super::super::support::drain_gateway(gateway_rx);
        let (actor, _) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
            gateway, persistence.handle.tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
        {
            let mut state = actor.state.lock().await;
            state.running_task = Some(super::super::super::support::running_task_stub("deferred-caller"));
            state.pending_notifications.extend([
                row("deferred-monitor", "deferred terminal evidence", NotificationSource::MonitorCompleted { task_id: "deferred-monitor".into() }),
                row("unrelated-monitor", "retained independent evidence", NotificationSource::MonitorEvent { task_id: "unrelated-monitor".into() }),
                row("deferred-monitor", "superseded deferred event", NotificationSource::MonitorEvent { task_id: "deferred-monitor".into() }),
                row("silent-deferred", "forbidden deferred terminal", NotificationSource::MonitorCompleted { task_id: "silent-deferred".into() }),
                row("silent-deferred", "superseded silent event", NotificationSource::MonitorEvent { task_id: "silent-deferred".into() }),
            ]);
        }
        assert_eq!(bounded(actor.consume_deferred_completions()).await, vec!["deferred-monitor", "silent-deferred"]);
        assert!(bounded(actor.consume_deferred_completions()).await.is_empty());
        let state = actor.state.lock().await;
        assert_eq!(state.pending_notifications.len(), 1);
        assert!(matches!(&state.pending_notifications[0].source, NotificationSource::MonitorEvent { task_id } if task_id == "unrelated-monitor"));
        assert!(SessionActor::queue_text_from_blocks(&state.pending_notifications[0].prompt_blocks).contains("retained independent evidence"));
        drop(state);
        let history = actor.chat_state_handle.snapshot().await.unwrap().conversation;
        assert_eq!(history.iter().filter(|item| item.text_content().contains("deferred terminal evidence")).count(), 1);
        assert!(history.iter().all(|item| !item.text_content().contains("superseded deferred event") && !item.text_content().contains("forbidden deferred terminal") && !item.text_content().contains("superseded silent event")));
        assert_eq!(storage.read_completion_state(&info).await.unwrap().receipts.iter().map(|receipt| receipt.completion.clone()).collect::<Vec<_>>(), vec![eligible]);
    }).await;
}
