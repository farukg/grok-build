use super::*;
use crate::session::acp_session::stop_gate::{TurnWaitDeclaration, WaitOnTarget};
use crate::session::acp_session::support;
use crate::session::commands::TaskCompletionBatch;
use crate::session::completion_delivery::{CompletionEventPolicy, CompletionPayloadSource};
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter};
use xai_grok_tools::computer::types::{TaskKind, TaskSnapshot};
use xai_grok_tools::types::completion_observation::{
    ObservedCompletion, ObservedTerminalCompletion,
};

fn snapshot(id: &str) -> TaskSnapshot {
    TaskSnapshot {
        task_id: id.into(),
        command: "printf result".into(),
        display_command: None,
        cwd: "/tmp".into(),
        start_time: std::time::SystemTime::UNIX_EPOCH,
        end_time: Some(std::time::SystemTime::UNIX_EPOCH),
        output: format!("result {id}"),
        output_file: Default::default(),
        truncated: false,
        exit_code: Some(0),
        signal: None,
        completed: true,
        kind: TaskKind::Bash,
        block_waited: false,
        explicitly_killed: false,
        kill_result_delivered: false,
        owner_session_id: Some("test-actor".into()),
        description: None,
        is_backgrounded: true,
        output_total_bytes: 0,
    }
}

struct Fixture {
    actor: Arc<SessionActor>,
    storage: Arc<JsonlStorageAdapter>,
    info: crate::session::info::Info,
    bridge: xai_grok_tools::notification::types::ToolNotificationHandle,
    gateway: mpsc::UnboundedReceiver<xai_acp_lib::AcpClientMessage>,
    _directory: tempfile::TempDir,
    _persistence: tokio_util::task::AbortOnDropHandle<()>,
    _receiver: tokio_util::task::AbortOnDropHandle<()>,
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("completion rendezvous")
}

impl Fixture {
    async fn new(tasks: &[TaskSnapshot]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::info::Info {
            id: acp::SessionId::new("test-actor"),
            cwd: "/tmp".into(),
        };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(
            directory.path().to_path_buf(),
        ));
        storage
            .init_session(&info, acp::ModelId::new("test"))
            .await
            .unwrap();
        for task in tasks {
            storage
                .enroll_terminal_completion(
                    &info,
                    CompletionPayloadSource::Task(Box::new(task.clone())),
                    format!("result {}", task.task_id),
                    CompletionEventPolicy::Automatic,
                )
                .await
                .unwrap();
        }
        let persistence = crate::session::persistence::durable_update_tests::test_actor(
            info.clone(),
            storage.clone(),
        );
        let (gateway, receiver) = mpsc::unbounded_channel();
        support::drain_gateway(receiver);
        let (mut actor, events) = support::create_test_actor_with_chat_persistence(
            0,
            256_000,
            85,
            gateway,
            persistence.handle.tx.clone(),
            Box::new(
                crate::session::chat_persistence::ChannelChatPersistence::new(
                    persistence.handle.tx.clone(),
                ),
            ),
        )
        .await;
        *actor.current_prompt_id.lock().unwrap() = Some("original-caller".into());
        actor.state.lock().await.running_task = Some(support::running_task_stub("original-caller"));
        let (mut config, publications, _, _) =
            crate::tools::notification_bridge::tests::make_test_config_full_raw();
        config.session_id = info.id.clone();
        config.persistence = persistence.handle.clone();
        config.goal_loop_active = actor.tool_context.goal_loop_active_gate.clone();
        actor.tool_context.task_completion_reservations =
            Some(config.task_completion_reservations.clone());
        let actor = Arc::new(actor);
        let (sender, receiver) = support::spawn_completion_receiver(actor.clone(), events);
        config.session_cmd_tx = sender;
        let bridge = crate::tools::notification_bridge::spawn_notification_bridge(config);
        Self {
            actor,
            storage,
            info,
            bridge,
            gateway: publications,
            _directory: directory,
            _persistence: tokio_util::task::AbortOnDropHandle::new(persistence.task),
            _receiver: receiver,
        }
    }

    async fn published(&mut self, count: usize) -> Vec<bool> {
        bounded(async {
            let mut verdicts = Vec::new();
            while verdicts.len() < count {
                if let Some(xai_acp_lib::AcpClientMessage::ExtNotification(message)) =
                    self.gateway.recv().await
                    && message.request.method.as_ref() == "x.ai/task_completed"
                {
                    let notification: crate::extensions::notification::SessionNotification =
                        serde_json::from_str(message.request.params.get()).unwrap();
                    if let crate::extensions::notification::SessionUpdate::TaskCompleted {
                        will_wake,
                        ..
                    } = notification.update
                    {
                        verdicts.push(will_wake);
                    }
                }
            }
            verdicts
        })
        .await
    }

    async fn consume(&self, task: &TaskSnapshot) {
        self.actor
            .chat_state_handle
            .push_assistant_response_and_ack(ConversationItem::assistant_tool_calls(vec![
                xai_grok_sampling_types::ToolCall {
                    id: format!("read-{}", task.task_id).into(),
                    name: "get_task_output".into(),
                    arguments: "{}".into(),
                },
            ]))
            .await
            .unwrap();
        bounded(self.actor.commit_observed_subagent_tool_result(
            ConversationItem::tool_result(
                format!("read-{}", task.task_id),
                format!("observed {}", task.task_id),
            ),
            vec![ObservedCompletion::Terminal(
                ObservedTerminalCompletion::Task {
                    snapshot: Box::new(task.clone()),
                },
            )],
        ))
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn consumed_result_published_after_final_answer_is_ui_only_even_with_a_stale_wait() {
    tokio::task::LocalSet::new().run_until(async {
        for final_answer in [false, true] {
            let task = snapshot("already-read");
            let mut fixture = Fixture::new(std::slice::from_ref(&task)).await;
            fixture.consume(&task).await;
            if final_answer {
                let admission = fixture.actor.current_turn_admission("original-caller").await.unwrap();
                fixture.storage.append_update_durable_commit_aware(&fixture.info, &crate::session::storage::SessionUpdate::Xai(Box::new(
                    crate::extensions::notification::SessionNotification {
                        session_id: fixture.info.id.clone(),
                        meta: Some(serde_json::json!({ "_grokParentTask": admission.parent_task, "_receiverTurnOrigin": crate::session::delivery_receiver::ReceiverTurnOrigin::Other })),
                        update: crate::extensions::notification::SessionUpdate::TurnCompleted {
                            prompt_id: "original-caller".into(), stop_reason: "end_turn".into(), agent_result: None,
                            error_kind: None, usage: None, elapsed_ms: None,
                        },
                    },
                ))).await.unwrap();
                fixture.actor.state.lock().await.running_task = None;
            }
            fixture.actor.record_turn_wait(TurnWaitDeclaration::of([WaitOnTarget::BashTask { task_id: task.task_id.clone() }]));
            let before = fixture.actor.chat_state_handle.snapshot().await.unwrap().conversation;
            fixture.bridge.send_task_complete(task.clone());
            assert_eq!(fixture.published(1).await, [false]);
            fixture.bridge.send_task_complete(task);
            assert_eq!(fixture.published(1).await, [false]);
            let state = fixture.actor.state.lock().await;
            assert!(state.pending_inputs.is_empty());
            assert!(state.pending_notifications.is_empty());
            if final_answer { assert!(state.running_task.is_none()); }
            drop(state);
            assert!(!fixture.actor.turn_wait_declaration.borrow().declaration.is_empty(), "consumed events do not satisfy a new wait");
            assert_eq!(fixture.actor.chat_state_handle.snapshot().await.unwrap().conversation.len(), before.len());
            let reopened = JsonlStorageAdapter::with_explicit_session_dir(fixture._directory.path().to_path_buf());
            let durable = reopened.read_completion_state(&fixture.info).await.unwrap();
            assert_eq!(durable.receipts.len(), 1);
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn jointly_available_outstanding_results_batch_and_duplicate_publication_cannot_wake_again() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let tasks = [snapshot("first"), snapshot("second"), snapshot("third")];
            let mut fixture = Fixture::new(&tasks).await;
            fixture
                .actor
                .record_turn_wait(TurnWaitDeclaration::of([WaitOnTarget::BashTask {
                    task_id: "second".into(),
                }]));
            for task in &tasks {
                fixture.bridge.send_task_complete(task.clone());
            }
            assert_eq!(fixture.published(3).await, [true, true, true]);
            assert!(
                fixture
                    .actor
                    .turn_wait_declaration
                    .borrow()
                    .declaration
                    .is_empty()
            );
            {
                let state = fixture.actor.state.lock().await;
                assert_eq!(state.pending_inputs.len(), 1);
                assert!(state.pending_inputs[0].verbatim);
                assert_eq!(state.pending_inputs[0].input_origin.completions().len(), 3);
            }
            for task in &tasks {
                fixture.bridge.send_task_complete(task.clone());
            }
            assert_eq!(fixture.published(3).await, [false, false, false]);
            assert!(bounded(fixture.actor.drain_terminal_wakes_at_safe_point()).await);
            assert!(!bounded(fixture.actor.drain_terminal_wakes_at_safe_point()).await);
            let history = fixture
                .actor
                .chat_state_handle
                .snapshot()
                .await
                .unwrap()
                .conversation;
            assert_eq!(history.len(), 1);
            for task in &tasks {
                assert_eq!(
                    history[0]
                        .text_content()
                        .matches(&format!("result {}", task.task_id))
                        .count(),
                    1
                );
            }
            assert_eq!(
                fixture
                    .storage
                    .read_completion_state(&fixture.info)
                    .await
                    .unwrap()
                    .receipts
                    .len(),
                3
            );
            for task in tasks {
                fixture.bridge.send_task_complete(task);
            }
            assert_eq!(fixture.published(3).await, [false, false, false]);
            assert!(fixture.actor.state.lock().await.pending_inputs.is_empty());
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn consuming_one_queued_batch_member_preserves_its_outstanding_peer() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let tasks = [snapshot("consumed-member"), snapshot("outstanding-member")];
            let mut fixture = Fixture::new(&tasks).await;
            for task in &tasks {
                fixture.bridge.send_task_complete(task.clone());
            }
            assert_eq!(fixture.published(2).await, [true, true]);
            fixture.consume(&tasks[0]).await;
            {
                let state = fixture.actor.state.lock().await;
                assert_eq!(state.pending_inputs.len(), 1);
                assert_eq!(state.pending_inputs[0].input_origin.completions().len(), 1);
            }
            assert!(bounded(fixture.actor.drain_terminal_wakes_at_safe_point()).await);
            let history = fixture
                .actor
                .chat_state_handle
                .snapshot()
                .await
                .unwrap()
                .conversation;
            let text = history
                .iter()
                .map(ConversationItem::text_content)
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(text.matches("observed consumed-member").count(), 1);
            assert_eq!(text.matches("result outstanding-member").count(), 1);
            assert!(!text.contains("result consumed-member"));
            assert_eq!(
                fixture
                    .storage
                    .read_completion_state(&fixture.info)
                    .await
                    .unwrap()
                    .receipts
                    .len(),
                2
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_committed_tool_receipt_racing_publication_is_not_a_live_recovery_wake() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let task = snapshot("receipt-race");
            let mut fixture = Fixture::new(std::slice::from_ref(&task)).await;
            fixture.bridge.send_task_complete(task.clone());
            assert_eq!(fixture.published(1).await, [true]);
            let id = fixture.actor.state.lock().await.pending_inputs[0]
                .input_origin
                .completions()[0]
                .clone();
            let admission = fixture
                .actor
                .current_turn_admission("original-caller")
                .await
                .unwrap();
            let mut projection =
                CompletionProjection::new(id, "original-caller".into(), String::new());
            projection.item =
                ConversationItem::tool_result("actual-racing-read", "racing read was consumed");
            projection.receipt.parent_task = admission.parent_task.clone();
            assert!(matches!(
                fixture
                    .actor
                    .chat_state_handle
                    .append_completion_and_ack(projection, admission)
                    .await
                    .unwrap(),
                CompletionProjectionAck::Appended
            ));
            assert_eq!(fixture.actor.state.lock().await.pending_inputs.len(), 1);
            assert!(!bounded(fixture.actor.drain_terminal_wakes_at_safe_point()).await);
            assert!(fixture.actor.state.lock().await.pending_inputs.is_empty());
            let history = fixture
                .actor
                .chat_state_handle
                .snapshot()
                .await
                .unwrap()
                .conversation;
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].text_content(), "racing read was consumed");
            assert_eq!(
                fixture
                    .storage
                    .read_completion_state(&fixture.info)
                    .await
                    .unwrap()
                    .receipts
                    .len(),
                1
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn an_uncommitted_result_and_speculative_reminder_cache_do_not_consume_the_wake() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let task = snapshot("cancelled-consumer");
            let mut fixture = Fixture::new(std::slice::from_ref(&task)).await;
            fixture
                .actor
                .mark_completions_reported(&[task.task_id.as_str()])
                .await;
            let id: CompletionIdentity = TerminalCompletionId::Task {
                owner_session_id: "test-actor".into(),
                task_id: task.task_id.clone(),
                task_kind: xai_chat_state::completion::CompletionTaskKind::Bash,
            }
            .into();
            let mut admission = fixture
                .actor
                .current_turn_admission("original-caller")
                .await
                .unwrap();
            admission.cancellation = tokio_util::sync::CancellationToken::new();
            admission.cancellation.cancel();
            let mut projection =
                CompletionProjection::new(id, "original-caller".into(), "not consumed".into());
            projection.item = ConversationItem::tool_result("cancelled-read", "not consumed");
            projection.receipt.parent_task = admission.parent_task.clone();
            assert!(matches!(
                fixture
                    .actor
                    .chat_state_handle
                    .append_completion_and_ack(projection, admission)
                    .await
                    .unwrap(),
                CompletionProjectionAck::TurnClosed
            ));
            assert!(
                fixture
                    .storage
                    .read_completion_state(&fixture.info)
                    .await
                    .unwrap()
                    .receipts
                    .is_empty()
            );
            fixture.bridge.send_task_complete(task);
            assert_eq!(fixture.published(1).await, [true]);
            assert!(bounded(fixture.actor.drain_terminal_wakes_at_safe_point()).await);
            let history = fixture
                .actor
                .chat_state_handle
                .snapshot()
                .await
                .unwrap()
                .conversation;
            assert_eq!(history.len(), 1);
            assert!(
                history[0]
                    .text_content()
                    .contains("result cancelled-consumer")
            );
            assert!(!history[0].text_content().contains("not consumed"));
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn outstanding_declared_goal_target_still_wakes_once_and_unselected_peer_stays_silent() {
    tokio::task::LocalSet::new().run_until(async {
        let tasks = [snapshot("selected"), snapshot("consumed-peer"), snapshot("unselected")];
        let mut fixture = Fixture::new(&tasks).await;
        fixture.consume(&tasks[1]).await;
        fixture.actor.goal_harness_enabled.store(true, std::sync::atomic::Ordering::Relaxed);
        fixture.actor.goal_tracker.lock().create_goal("goal".into(), "wait for selected result".into(), None, 0, chrono::Utc::now().to_rfc3339(), None);
        fixture.actor.set_goal_loop_active_resource(true).await;
        fixture.actor.record_turn_wait(TurnWaitDeclaration::of([
            WaitOnTarget::BashTask { task_id: "selected".into() },
            WaitOnTarget::BashTask { task_id: "consumed-peer".into() },
        ]));
        for index in [1, 2, 0] { fixture.bridge.send_task_complete(tasks[index].clone()); }
        assert_eq!(fixture.published(3).await, [false, false, true]);
        assert!(fixture.actor.turn_wait_declaration.borrow().declaration.is_empty());
        {
            let state = fixture.actor.state.lock().await;
            assert_eq!(state.pending_inputs.len(), 1);
            assert_eq!(state.pending_inputs[0].input_origin.completions().len(), 1);
            assert!(state.pending_notifications.is_empty());
        }
        assert!(bounded(fixture.actor.drain_terminal_wakes_at_safe_point()).await);
        assert!(!bounded(fixture.actor.drain_terminal_wakes_at_safe_point()).await);
        for task in &tasks { fixture.bridge.send_task_complete(task.clone()); }
        assert_eq!(fixture.published(3).await, [false, false, false]);
        fixture.actor.goal_harness_enabled.store(false, std::sync::atomic::Ordering::Relaxed);
        fixture.actor.set_goal_loop_active_resource(false).await;
        fixture.actor.record_turn_wait(TurnWaitDeclaration::of([WaitOnTarget::BashTask { task_id: "unselected".into() }]));
        for task in &tasks { fixture.bridge.send_task_complete(task.clone()); }
        assert_eq!(fixture.published(3).await, [false, false, false]);
        assert!(!fixture.actor.turn_wait_declaration.borrow().declaration.is_empty(), "durable goal suppression cannot satisfy a later wait");
        let state = fixture.actor.state.lock().await;
        assert!(state.pending_inputs.is_empty());
        assert!(state.pending_notifications.is_empty());
        drop(state);
        let history = fixture.actor.chat_state_handle.snapshot().await.unwrap().conversation;
        let text = history.iter().map(ConversationItem::text_content).collect::<Vec<_>>().join("\n");
        assert_eq!(text.matches("result selected").count(), 1);
        assert_eq!(text.matches("observed consumed-peer").count(), 1);
        assert!(!text.contains("result consumed-peer"));
        assert!(!text.contains("result unselected"));
        let reopened = JsonlStorageAdapter::with_explicit_session_dir(fixture._directory.path().to_path_buf());
        let durable = reopened.read_completion_state(&fixture.info).await.unwrap();
        assert_eq!(durable.receipts.len(), 2);
        assert!(matches!(&durable.obligations.iter().find(|obligation| matches!(obligation.id(),
            CompletionIdentity::V2 { terminal: TerminalCompletionId::Task { task_id, .. }, .. } if task_id == "unselected")).unwrap(),
            CompletionObligation::Suppressed { reason: crate::session::completion_delivery::CompletionSuppression::GoalNotificationPolicy,
                source: crate::session::completion_delivery::SuppressedCompletionSource::Enrolled(_), .. }));
        assert_eq!(durable.obligations.iter().filter(|obligation| matches!(obligation, CompletionObligation::Suppressed { .. })).count(), 1);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn human_stop_parks_the_whole_durable_batch_without_consuming_it() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let tasks = [snapshot("parked-first"), snapshot("parked-second")];
            let fixture = Fixture::new(&tasks).await;
            fixture
                .actor
                .state
                .lock()
                .await
                .pending_inputs
                .push_front(support::user_item("original-caller", "test-client"));
            let mut admissions = Vec::new();
            let mut receipts = Vec::new();
            for task in tasks {
                let (admission, receipt) =
                    support::task_wake_admission(NotificationSource::BashTaskCompleted {
                        task_id: task.task_id,
                    });
                admissions.push(admission);
                receipts.push(receipt);
            }
            let (respond_to, _) = oneshot::channel();
            bounded(
                fixture
                    .actor
                    .admit_task_completion_batch(TaskCompletionBatch {
                        prompt_id: "notifications-before-stop".into(),
                        automatic: true,
                        admissions,
                        respond_to,
                    }),
            )
            .await;
            for receipt in receipts {
                assert!(receipt.await.unwrap());
            }
            let outcome = bounded(fixture.actor.cancel_running_task(
                crate::session::commands::CancelOptions {
                    trigger: Some(crate::session::commands::CancelTrigger::Esc),
                    user_initiated: true,
                    ..Default::default()
                },
            ))
            .await;
            assert!(outcome.settled);
            assert!(matches!(
                outcome.barrier,
                crate::session::acp_session::cancel::WakeBarrier::Armed
            ));
            let (completion_tx, _) = mpsc::unbounded_channel();
            bounded(SessionActor::maybe_start_running_task(
                fixture.actor.clone(),
                completion_tx,
            ))
            .await;
            let state = fixture.actor.state.lock().await;
            assert!(state.running_task.is_none());
            assert!(state.notifications_suppressed);
            assert_eq!(state.pending_inputs.len(), 1);
            assert_eq!(state.pending_inputs[0].input_origin.completions().len(), 2);
            drop(state);
            assert!(
                fixture
                    .storage
                    .read_completion_state(&fixture.info)
                    .await
                    .unwrap()
                    .receipts
                    .is_empty()
            );
        })
        .await;
}
