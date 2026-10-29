use super::support::*;
use super::*;
use crate::extensions::interject::{
    QueueAction, QueueCaller, QueueOperation, QueueRejection, QueueRequest, QueueResponse,
    QueueResult,
};
use std::rc::Rc;

fn request(id: &str, version: u64, owner: Option<&str>) -> QueueRequest {
    QueueRequest {
        operation: QueueOperation {
            session_id: "test-actor".into(),
            id: id.into(),
            expected_version: version,
            operation_id: uuid::Uuid::new_v4(),
        },
        action: QueueAction::Prioritize { new_text: None },
        owner: owner.map(str::to_owned),
    }
}

async fn submit(
    sender: &mpsc::UnboundedSender<SessionCommand>,
    request: QueueRequest,
) -> QueueResponse {
    submit_as(sender, QueueCaller::for_test_session("test-actor"), request).await
}

async fn submit_as(
    sender: &mpsc::UnboundedSender<SessionCommand>,
    caller: QueueCaller,
    request: QueueRequest,
) -> QueueResponse {
    let (respond_to, receive) = oneshot::channel();
    sender
        .send(SessionCommand::QueueSendNow {
            caller,
            request,
            respond_to,
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), receive)
        .await
        .unwrap()
        .unwrap()
}

fn hold_next_flush(actor: &mut Arc<SessionActor>) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
    let actor = Arc::get_mut(actor).unwrap();
    let upstream = actor.notifications.persistence_tx.clone();
    let (proxy, mut commands) = mpsc::unbounded_channel();
    actor.notifications.persistence_tx = proxy;
    let (observed, committed) = oneshot::channel();
    let (release, released) = oneshot::channel();
    tokio::task::spawn_local(async move {
        let mut barrier = Some((observed, released));
        while let Some(command) = commands.recv().await {
            let PersistenceMsg::FlushAndAck { respond_to } = command else {
                upstream.send(command).unwrap();
                continue;
            };
            let (ack, persisted) = oneshot::channel();
            upstream
                .send(PersistenceMsg::FlushAndAck { respond_to: ack })
                .unwrap();
            let result = persisted.await.unwrap();
            assert!(result.is_ok());
            if let Some((observed, released)) = barrier.take() {
                observed.send(()).unwrap();
                released.await.unwrap();
            }
            let _ = respond_to.send(result);
        }
    });
    (committed, release)
}

async fn actor() -> (
    Arc<SessionActor>,
    mpsc::UnboundedReceiver<SessionEvent>,
    mpsc::UnboundedReceiver<ChatFixtureEvent>,
) {
    let (gateway_tx, gateway_rx) = mpsc::unbounded_channel();
    spawn_gateway_loop(gateway_rx);
    let (actor, events, writes) =
        create_observed_completion_test_actor(0, 256_000, 85, gateway_tx).await;
    (Arc::new(actor), events, writes)
}

#[tokio::test(flavor = "current_thread")]
async fn correlated_priority_is_queued_under_finalization_then_projects_exact_payload_once() {
    tokio::task::LocalSet::new().run_until(async {
        let (mut actor, events, mut writes) = actor().await;
        let (committed, release_flush) = hold_next_flush(&mut actor);
        let (mut input, _terminal) = user_item_with_rx("human-held", "A");
        let (ready, publication) = oneshot::channel();
        let (persist_ack, persisted) = oneshot::channel();
        input.initial_child_prompt_ready = Some(crate::session::message_delivery::InitialChildPromptSink::Prompt(ready));
        input.persist_ack = Some(persist_ack);
        let mut lease = {
            let mut state = actor.state.lock().await;
            state.pending_inputs.push_back(input);
            state.notifications_suppressed = true;
            state.arm_hook_block_hold();
            state.claim_cancel_finalization().unwrap()
        };
        actor.tool_context.task_wake_suppressed.as_ref().unwrap().set(true);
        let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
        let (admission, admitted) = task_wake_admission(NotificationSource::BashTaskCompleted {
            task_id: "manual-stop-completion".into(),
        });
        let (respond_to, terminal) = oneshot::channel();
        sender.send(SessionCommand::Prompt {
            prompt_id: "task-completed-manual-stop-completion".into(),
            prompt_blocks: vec![acp::ContentBlock::from("Background task manual-stop-completion completed.")],
            prompt_mode: PromptMode::Agent,
            artifact_upload_ctx: None,
            client_identifier: None,
            screen_mode: None,
            verbatim: true,
            traceparent: None,
            json_schema: None,
            send_now: false,
            admission: Some(admission),
            tool_overrides_update: None,
            respond_to,
            prompt_admitted: None,
            persist_ack: None,
            parsed_prompt_tx: None,
        }).unwrap();
        assert!(!tokio::time::timeout(Duration::from_secs(5), admitted).await.unwrap().unwrap());
        assert!(matches!(tokio::time::timeout(Duration::from_secs(5), terminal).await.unwrap().unwrap().unwrap().completion_kind,
            PromptCompletionKind::RemovedFromQueue));
        let sent = request("human-held", 0, Some("A"));
        let accepted = submit(&sender, sent.clone()).await;
        assert_eq!(accepted.operation, sent.operation);
        assert!(matches!(&accepted.result, QueueResult::Queued { row } if row.id == "human-held" && row.text == "text for human-held"));
        assert_eq!(submit(&sender, sent.clone()).await, accepted, "duplicate operation cannot mutate twice");
        let mut observe = sent.clone();
        observe.action = QueueAction::Observe;
        assert!(matches!(submit(&sender, observe.clone()).await.result, QueueResult::Queued { .. }));
        {
            let state = actor.state.lock().await;
            assert!(state.finalization_gate.is_active());
            assert!(state.running_task.is_none());
            assert!(!state.hook_block_held(), "explicit accepted send-now retains the existing hook release");
            assert!(state.notifications_suppressed, "priority acceptance is not blanket wake release");
            assert!(matches!(state.pending_notifications.as_slice(), [PendingNotification {
                prompt_id, source: NotificationSource::BashTaskCompleted { task_id }, ..
            }] if prompt_id == "task-completed-manual-stop-completion" && task_id == "manual-stop-completion"));
            assert_eq!(state.pending_inputs.len(), 1);
            assert!(state.pending_inputs[0].input_origin.policy().authority.is_human_intent());
        }
        assert!(actor.tool_context.task_wake_suppressed.as_ref().unwrap().get());
        assert!(actor.chat_state_handle.get_conversation().await.iter().all(|item| !item.is_human_user_turn()));
        assert!(actor.state.lock().await.finish_finalization(&mut lease));
        sender.send(SessionCommand::ReleaseEdit { id: "human-held".into() }).unwrap();
        let crate::session::message_delivery::InitialChildPublication::Prompt(release) =
            tokio::time::timeout(Duration::from_secs(5), publication).await.unwrap().unwrap()
        else { panic!("human queue promotion must keep its ordinary publication gate"); };
        let identity = actor.state.lock().await.running_task.as_ref().unwrap().identity.clone();
        assert!(matches!(submit(&sender, observe.clone()).await.result, QueueResult::Unconfirmed), "a startgate reservation is not committed input");
        assert_eq!(submit(&sender, sent.clone()).await, accepted);
        assert!(Rc::ptr_eq(&identity, &actor.state.lock().await.running_task.as_ref().unwrap().identity));
        {
            let state = actor.state.lock().await;
            assert_eq!(state.pending_notifications.len(), 1, "human priority must retain the stopped completion obligation");
            assert!(matches!(state.pending_notifications[0].source,
                NotificationSource::BashTaskCompleted { ref task_id } if task_id == "manual-stop-completion"));
            assert!(state.notifications_suppressed);
        }
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), committed).await.unwrap().unwrap();
        assert!(matches!(submit(&sender, observe.clone()).await.result,
            QueueResult::Started { prompt_id } if prompt_id == "human-held"));
        for wrong_identity in [false, true] {
            let mut conflict = observe.clone();
            if wrong_identity {
                conflict.operation.expected_version += 1;
            } else {
                conflict.owner = Some("different-owner".into());
            }
            assert!(matches!(submit(&sender, conflict).await.result,
                QueueResult::Rejected { reason: QueueRejection::OperationConflict, .. }));
        }
        release_flush.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), persisted).await.unwrap().unwrap();
        let history = actor.chat_state_handle.get_conversation().await;
        assert_eq!(history.iter().filter(|item| item.is_human_user_turn() && item.text_content().contains("text for human-held")).count(), 1);
        assert_eq!(submit(&sender, sent).await, accepted, "a post-projection duplicate must not requeue input");
        let history = actor.chat_state_handle.get_conversation().await;
        assert_eq!(history.iter().filter(|item| item.is_human_user_turn() && item.text_content().contains("text for human-held")).count(), 1);
        let mut persisted_inputs = 0;
        while let Ok(event) = writes.try_recv() {
            match event {
                ChatFixtureEvent::MessageQueued(item) => {
                    if item.is_human_user_turn() && item.text_content().contains("text for human-held") {
                        persisted_inputs += 1;
                    }
                }
                ChatFixtureEvent::InputAcknowledged { .. } => {}
            }
        }
        assert_eq!(persisted_inputs, 1, "one actual user append, not a response-only success oracle");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn actor_independently_rejects_running_missing_stale_owner_and_protected_origins() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, events, _) = actor().await;
        {
            let mut state = actor.state.lock().await;
            state.pending_inputs.push_back(user_item("running", "A"));
            state.running_task = Some(running_task_stub("running"));
            state.pending_inputs.push_back(user_item("owned", "A"));
            for (id, origin) in [
                ("parent-agent", PromptOrigin::ParentAgentMessage { message_id: "agent-message".into(), sender_session_id: "parent".into() }),
                ("parent-human", PromptOrigin::ParentHumanMessage { message_id: "human-message".into(), sender_session_id: "parent".into() }),
                ("delivery", PromptOrigin::Delivery { delivery_id: "delivery".into(), payload_digest: "fixture-digest".into() }),
            ] {
                let mut input = user_item(id, "A");
                input.input_origin = InputOrigin::new(origin);
                input.queue_mutation_policy = QueueMutationPolicy::from_input_origin(&input.input_origin);
                state.pending_inputs.push_back(input);
            }
            let (hidden, _) = input_with_origin_rx("task-completed-held", PromptOrigin::TaskCompleted { task_id: "held".into() });
            state.pending_inputs.push_back(hidden);
        }
        let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
        for (id, version, owner, expected) in [
            ("running", 0, Some("A"), QueueRejection::Running),
            ("missing", 0, Some("A"), QueueRejection::Missing),
            ("owned", 1, Some("A"), QueueRejection::Stale),
            ("owned", 0, Some("B"), QueueRejection::NotOwner),
            ("owned", 0, None, QueueRejection::NotOwner),
            ("parent-agent", 0, Some("A"), QueueRejection::Protected),
            ("parent-human", 0, Some("A"), QueueRejection::Protected),
            ("delivery", 0, Some("A"), QueueRejection::Protected),
            ("task-completed-held", 0, None, QueueRejection::Protected),
        ] {
            let sent = request(id, version, owner);
            let response = submit(&sender, sent.clone()).await;
            assert_eq!(response.operation, sent.operation);
            assert!(matches!(response.result, QueueResult::Rejected { reason, .. } if reason == expected));
        }
        let mut wrong_session = request("owned", 0, Some("A"));
        wrong_session.operation.session_id = "different-session".into();
        assert!(matches!(submit(&sender, wrong_session).await.result, QueueResult::Rejected { reason: QueueRejection::WrongSession, .. }));
        let state = actor.state.lock().await;
        assert_eq!(state.running_prompt_id(), Some("running"));
        assert_eq!(state.pending_inputs.len(), 6);
        assert!(!actor.build_queue_wire(&state).iter().any(|row| row.id == "task-completed-held"));
        assert_eq!(state.pending_inputs[1].queue_meta.as_ref().unwrap().version, 0);
        assert!(actor.pending_interjections.is_empty());
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn rejected_hidden_completion_keeps_manual_stop_obligation_and_wake_barrier() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, events, _) = actor().await;
        {
            let mut state = actor.state.lock().await;
            let (input, _) = input_with_origin_rx("task-completed-held", PromptOrigin::TaskCompleted { task_id: "held".into() });
            state.pending_inputs.push_back(input);
            state.notifications_suppressed = true;
        }
        actor.tool_context.task_wake_suppressed.as_ref().unwrap().set(true);
        let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
        let response = submit(&sender, request("task-completed-held", 0, None)).await;
        assert!(matches!(response.result, QueueResult::Rejected { reason: QueueRejection::Protected, row: None }));
        let state = actor.state.lock().await;
        assert!(state.running_task.is_none());
        assert!(state.notifications_suppressed);
        assert!(actor.tool_context.task_wake_suppressed.as_ref().unwrap().get());
        assert_eq!(state.pending_inputs.len(), 1);
        assert!(matches!(state.pending_inputs[0].input_origin.as_prompt_origin(), PromptOrigin::TaskCompleted { task_id } if task_id == "held"));
        assert!(actor.build_queue_wire(&state).is_empty());
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn queue_operation_observes_without_reapplication_or_cross_version_reuse() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, events, _) = actor().await;
        actor.compaction.cancel.seal_manual();
        actor.state.lock().await.pending_inputs.push_back(user_item("held", "A"));
        let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
        let sent = request("held", 0, Some("A"));
        let accepted = submit(&sender, sent.clone()).await;
        assert!(matches!(accepted.result, QueueResult::Queued { .. }));
        assert!(actor.state.lock().await.running_task.is_none());
        assert!(actor.compaction.cancel.manual_is_sealed());
        let mut observe = sent.clone();
        observe.action = QueueAction::Observe;
        assert_eq!(submit(&sender, observe.clone()).await, accepted);
        let mut conflict = sent.clone();
        conflict.operation.expected_version = 1;
        assert!(matches!(submit(&sender, conflict).await.result, QueueResult::Rejected { reason: QueueRejection::OperationConflict, .. }));
        let mut unknown = observe.clone();
        unknown.operation.operation_id = uuid::Uuid::new_v4();
        assert!(matches!(submit(&sender, unknown).await.result, QueueResult::Unconfirmed));
        sender.send(SessionCommand::EditQueuedPrompt { id: "held".into(), new_text: "new authoritative payload".into(), editor: Some("A".into()) }).unwrap();
        assert!(matches!(submit(&sender, observe).await.result, QueueResult::Rejected { reason: QueueRejection::Stale, row: Some(row) } if row.version == 1 && row.text == "new authoritative payload"));
        assert_eq!(submit(&sender, sent).await, accepted);
        let state = actor.state.lock().await;
        assert_eq!(state.pending_inputs.len(), 1);
        assert_eq!(state.pending_inputs[0].queue_meta.as_ref().unwrap().version, 1);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn goal_interjection_reports_its_real_running_binding_without_second_turn() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (actor, events, _) = actor().await;
            actor.goal_tracker.lock().create_goal(
                "goal".into(),
                "objective".into(),
                None,
                0,
                chrono::Utc::now().to_rfc3339(),
                None,
            );
            {
                let mut state = actor.state.lock().await;
                state
                    .pending_inputs
                    .push_back(user_item("running-goal", "A"));
                state.running_task = Some(running_task_stub("running-goal"));
                state.front_message_committed = true;
                state.pending_inputs.push_back(user_item("steer", "A"));
            }
            let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
            let sent = request("steer", 0, Some("A"));
            let response = submit(&sender, sent.clone()).await;
            assert_eq!(
                response.result,
                QueueResult::GoalInterjection {
                    running_prompt_id: "running-goal".into()
                }
            );
            assert_eq!(submit(&sender, sent).await, response);
            let state = actor.state.lock().await;
            assert_eq!(state.running_prompt_id(), Some("running-goal"));
            assert_eq!(state.pending_inputs.len(), 1);
            drop(state);
            let interjections = actor.pending_interjections.drain_all();
            assert_eq!(interjections.len(), 1);
            assert_eq!(interjections[0].text, "text for steer");
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn finalizing_goal_keeps_priority_queued_instead_of_claiming_a_goal_binding() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (actor, events, _) = actor().await;
            actor.goal_tracker.lock().create_goal(
                "goal".into(),
                "objective".into(),
                None,
                0,
                chrono::Utc::now().to_rfc3339(),
                None,
            );
            let mut lease = {
                let mut state = actor.state.lock().await;
                state
                    .pending_inputs
                    .push_back(user_item("running-goal", "A"));
                state.running_task = Some(running_task_stub("running-goal"));
                state.front_message_committed = true;
                state.pending_inputs.push_back(user_item("steer", "A"));
                state.claim_cancel_finalization().unwrap()
            };
            let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
            let response = submit(&sender, request("steer", 0, Some("A"))).await;
            assert!(matches!(response.result, QueueResult::Queued { row } if row.id == "steer"));
            assert!(actor.pending_interjections.is_empty());
            let mut state = actor.state.lock().await;
            assert_eq!(state.running_prompt_id(), Some("running-goal"));
            assert_eq!(state.pending_inputs.len(), 2);
            assert!(state.release_finalizing_task(&lease));
            assert!(state.finish_finalization(&mut lease));
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn rejected_operation_cannot_release_a_held_human_input() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, events, _) = actor().await;
        {
            let mut state = actor.state.lock().await;
            state.pending_inputs.push_back(user_item("held", "A"));
            state.arm_hook_block_hold();
            state.notifications_suppressed = true;
        }
        actor.tool_context.task_wake_suppressed.as_ref().unwrap().set(true);
        let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
        let sent = request("held", 1, Some("A"));
        let rejected = submit(&sender, sent.clone()).await;
        assert!(matches!(&rejected.result, QueueResult::Rejected { reason: QueueRejection::Stale, row: Some(row) } if row.version == 0));
        assert_eq!(submit(&sender, sent).await, rejected);
        let state = actor.state.lock().await;
        assert!(state.hook_block_held());
        assert!(state.notifications_suppressed);
        assert!(actor.tool_context.task_wake_suppressed.as_ref().unwrap().get());
        assert!(state.running_task.is_none());
        assert_eq!(state.pending_inputs.len(), 1);
        assert!(!state.pending_inputs[0].send_now);
        assert_eq!(state.pending_inputs[0].queue_meta.as_ref().unwrap().text, "text for held");
        drop(state);
        assert!(actor.chat_state_handle.get_conversation().await.iter().all(|item| !item.is_human_user_turn()));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn lost_reply_observes_the_atomic_edit_without_reapplying_it() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, events, _) = actor().await;
        actor.compaction.cancel.seal_manual();
        actor.state.lock().await.pending_inputs.push_back(user_item("held", "A"));
        let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
        let mut sent = request("held", 0, Some("A"));
        sent.action = QueueAction::Prioritize { new_text: Some("edited exact payload".into()) };
        let (respond_to, receive) = oneshot::channel();
        drop(receive);
        sender.send(SessionCommand::QueueSendNow { caller: QueueCaller::for_test_session("test-actor"), request: sent.clone(), respond_to }).unwrap();
        let mut observe = sent.clone();
        observe.action = QueueAction::Observe;
        let accepted = submit(&sender, observe).await;
        assert_eq!(accepted.operation, sent.operation);
        assert!(matches!(&accepted.result, QueueResult::Queued { row } if row.version == 1 && row.text == "edited exact payload"));
        assert_eq!(submit(&sender, sent).await, accepted);
        let state = actor.state.lock().await;
        assert!(state.running_task.is_none());
        assert_eq!(state.pending_inputs.len(), 1);
        let input = &state.pending_inputs[0];
        assert!(input.send_now);
        assert!(input.input_origin.policy().authority.is_human_intent());
        assert_eq!(input.queue_meta.as_ref().unwrap().version, 1);
        assert_eq!(input.queue_meta.as_ref().unwrap().text, "edited exact payload");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn copied_owner_and_operation_do_not_delegate_session_control() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (actor, events, _) = actor().await;
            let _lease = {
                let mut state = actor.state.lock().await;
                state.pending_inputs.push_back(user_item("owned", "A"));
                state.claim_cancel_finalization().unwrap()
            };
            let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
            let accepted_request = request("owned", 0, Some("A"));
            let accepted = submit(&sender, accepted_request.clone()).await;
            assert!(matches!(&accepted.result, QueueResult::Queued { .. }));
            {
                let mut state = actor.state.lock().await;
                state.arm_hook_block_hold();
                state.notifications_suppressed = true;
            }
            let before = actor.build_queue_wire(&*actor.state.lock().await);
            let mut spoof = request("owned", 0, Some("A"));
            spoof.action = QueueAction::Prioritize {
                new_text: Some("forged payload".into()),
            };
            let mut observe = accepted_request.clone();
            observe.action = QueueAction::Observe;
            for attempt in [spoof, observe.clone()] {
                let response = submit_as(
                    &sender,
                    QueueCaller::for_test_session("not-delegated-session"),
                    attempt,
                )
                .await;
                assert!(matches!(
                    response.result,
                    QueueResult::Rejected {
                        reason: QueueRejection::Unauthorized,
                        row: None
                    }
                ));
            }
            let state = actor.state.lock().await;
            assert_eq!(actor.build_queue_wire(&state), before);
            assert!(state.hook_block_held());
            assert!(state.notifications_suppressed);
            assert!(state.finalization_gate.is_active());
            assert!(state.running_task.is_none());
            assert!(actor.pending_interjections.is_empty());
            drop(state);
            assert_eq!(submit(&sender, accepted_request).await, accepted);
            assert_eq!(submit(&sender, observe).await, accepted);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn count_capacity_refuses_new_operations_but_keeps_known_receipts() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (actor, events, _) = actor().await;
            let _lease = {
                let mut state = actor.state.lock().await;
                state.pending_inputs.push_back(user_item("owned", "A"));
                state.arm_hook_block_hold();
                state.claim_cancel_finalization().unwrap()
            };
            let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
            let first = request("missing", 0, Some("A"));
            let receipt = submit(&sender, first.clone()).await;
            assert!(matches!(
                &receipt.result,
                QueueResult::Rejected {
                    reason: QueueRejection::Missing,
                    ..
                }
            ));
            for index in 1..=super::queue_mutation::QUEUE_OPERATION_LIMIT {
                let mut attempt = request(
                    if index % 2 == 0 { "missing" } else { "owned" },
                    9,
                    Some("A"),
                );
                attempt.action = QueueAction::Prioritize {
                    new_text: Some("ø".repeat(16 * 1024)),
                };
                let result = submit(&sender, attempt).await.result;
                if index == super::queue_mutation::QUEUE_OPERATION_LIMIT {
                    assert!(matches!(
                        result,
                        QueueResult::Rejected {
                            reason: QueueRejection::Capacity,
                            row: None
                        }
                    ));
                } else {
                    assert!(matches!(
                        result,
                        QueueResult::Rejected {
                            reason: QueueRejection::Missing | QueueRejection::Stale,
                            ..
                        }
                    ));
                }
            }
            let mut mutation = request("owned", 0, Some("A"));
            mutation.action = QueueAction::Prioritize {
                new_text: Some("must not replace".into()),
            };
            assert!(matches!(
                submit(&sender, mutation).await.result,
                QueueResult::Rejected {
                    reason: QueueRejection::Capacity,
                    row: None
                }
            ));
            assert_eq!(submit(&sender, first.clone()).await, receipt);
            let mut observe = first;
            observe.action = QueueAction::Observe;
            assert_eq!(submit(&sender, observe).await, receipt);
            let mut unknown = request("owned", 0, Some("A"));
            unknown.action = QueueAction::Observe;
            assert_eq!(
                submit(&sender, unknown).await.result,
                QueueResult::Unconfirmed
            );
            let state = actor.state.lock().await;
            assert_eq!(
                state.pending_inputs[0].queue_meta.as_ref().unwrap().text,
                "text for owned"
            );
            assert_eq!(
                state.pending_inputs[0].queue_meta.as_ref().unwrap().version,
                0
            );
            assert!(state.hook_block_held());
            assert!(state.running_task.is_none());
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn byte_capacity_is_reserved_before_atomic_edit_and_keeps_last_receipt() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (actor, events, _) = actor().await;
            let _lease = {
                let mut state = actor.state.lock().await;
                state.pending_inputs.push_back(user_item("owned", "A"));
                state.claim_cancel_finalization().unwrap()
            };
            let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
            let mut version = 0;
            let mut last = None;
            for _ in 0..20 {
                let mut attempt = request("owned", version, Some("A"));
                attempt.action = QueueAction::Prioritize {
                    new_text: Some("界".repeat(350_000)),
                };
                let response = submit(&sender, attempt.clone()).await;
                match &response.result {
                    QueueResult::Queued { row } => {
                        version += 1;
                        assert_eq!(row.version, version);
                        last = Some((attempt, response));
                        actor.state.lock().await.arm_hook_block_hold();
                    }
                    QueueResult::Rejected { reason, row } => {
                        assert_eq!(*reason, QueueRejection::Capacity);
                        assert!(row.is_none());
                        break;
                    }
                    QueueResult::GoalInterjection { .. }
                    | QueueResult::Started { .. }
                    | QueueResult::Unconfirmed => {
                        panic!("an unstarted held input must not claim a running binding");
                    }
                }
            }
            assert!(
                (1..16).contains(&version),
                "UTF-8 bytes, not character count, consume the byte budget"
            );
            let (accepted, receipt) = last.unwrap();
            let before = actor.build_queue_wire(&*actor.state.lock().await);
            assert_eq!(submit(&sender, accepted.clone()).await, receipt);
            let mut observe = accepted.clone();
            observe.action = QueueAction::Observe;
            assert_eq!(submit(&sender, observe).await, receipt);
            let mut oversized = request("owned", version, Some("A"));
            oversized.action = QueueAction::Prioritize {
                new_text: Some(
                    "界".repeat(crate::extensions::interject::MAX_QUEUE_REQUEST_BYTES / 3 + 1),
                ),
            };
            assert!(matches!(
                submit(&sender, oversized).await.result,
                QueueResult::Rejected {
                    reason: QueueRejection::PayloadTooLarge,
                    row: None
                }
            ));
            let state = actor.state.lock().await;
            assert_eq!(actor.build_queue_wire(&state), before);
            assert!(state.hook_block_held());
            assert!(state.finalization_gate.is_active());
            assert!(state.running_task.is_none());
        })
        .await;
}

async fn leader_request(
    client: &mut crate::leader::LeaderClient,
    request: &QueueRequest,
    claimed_client: &str,
) -> QueueResponse {
    let id = uuid::Uuid::new_v4().to_string();
    let mut params = serde_json::to_value(request).unwrap();
    params.as_object_mut().unwrap().insert(
        "_meta".into(),
        serde_json::json!({
            "clientIdentifier": claimed_client,
            "caller": claimed_client,
            "sessionControl": { "sessionId": "claimed-grant" },
        }),
    );
    client
        .send(
            serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": "_x.ai/queue/interject", "params": params,
            })
            .to_string(),
        )
        .unwrap();
    loop {
        let payload = tokio::time::timeout(Duration::from_secs(5), client.recv())
            .await
            .unwrap()
            .unwrap();
        let reply: serde_json::Value = serde_json::from_str(&payload).unwrap();
        if reply.get("id").and_then(serde_json::Value::as_str) == Some(id.as_str()) {
            return serde_json::from_value(reply.get("result").unwrap().clone()).unwrap();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn two_leader_clients_share_real_session_control_not_claimed_client_ownership() {
    use crate::leader::{ClientCapabilities, ClientMode, LeaderClient};
    use acp::Agent as _;
    let local = tokio::task::LocalSet::new();
    let keepalive = local.run_until(async {
        let (actor, events, _) = actor().await;
        let _lease = {
            let mut state = actor.state.lock().await;
            state.pending_inputs.push_back(user_item("owned", "A"));
            state.claim_cancel_finalization().unwrap()
        };
        let (sender, _loop_task) = spawn_completion_receiver(actor.clone(), events);
        let agent = Rc::new(crate::agent::mvp_agent::tests::build_minimal_agent_for_tests());
        let mut handle = crate::agent::mvp_agent::tests::make_test_handle("test", false, Some("A"));
        handle.info.id = actor.session_info.id.clone();
        handle.cmd_tx = sender.clone();
        agent.insert_resident(&handle.info.id.clone(), handle);
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("queue.sock");
        let server = crate::leader::spawn_leader_server(socket.clone()).await.unwrap();
        let mut incoming = server.acp_rx;
        let responses = server.response_tx;
        let serving = agent.clone();
        tokio::task::spawn_local(async move {
            #[derive(serde::Deserialize)]
            struct ForwardedRequest {
                id: serde_json::Value,
                method: String,
                params: Box<serde_json::value::RawValue>,
            }
            while let Some(payload) = incoming.recv().await {
                let forwarded: ForwardedRequest = serde_json::from_str(&payload).unwrap();
                assert_eq!(forwarded.method, "_x.ai/queue/interject");
                let response = serving.ext_method(acp::ExtRequest::new(
                    "x.ai/queue/interject", forwarded.params.into(),
                )).await.unwrap();
                let result: serde_json::Value = serde_json::from_str(response.0.get()).unwrap();
                responses.send(serde_json::json!({
                    "jsonrpc": "2.0", "id": forwarded.id, "result": result,
                }).to_string()).unwrap();
            }
        });
        let mut a = LeaderClient::connect(socket.clone(), "operator-A", ClientMode::Stdio, ClientCapabilities::default()).await.unwrap();
        let mut b = LeaderClient::connect(socket, "operator-B", ClientMode::Stdio, ClientCapabilities::default()).await.unwrap();
        let mut original = request("owned", 0, Some("A"));
        original.action = QueueAction::Prioritize { new_text: Some("operator A edit".into()) };
        let accepted = leader_request(&mut a, &original, "A").await;
        assert!(matches!(&accepted.result, QueueResult::Queued { row } if row.version == 1 && row.text == "operator A edit"));
        let mut observe = original.clone();
        observe.action = QueueAction::Observe;
        assert_eq!(leader_request(&mut b, &observe, "forged-A").await, accepted);
        let mut delegated = request("owned", 1, Some("A"));
        delegated.action = QueueAction::Prioritize { new_text: Some("authorized operator B edit".into()) };
        let delegated_receipt = leader_request(&mut b, &delegated, "forged-A").await;
        assert!(matches!(&delegated_receipt.result, QueueResult::Queued { row } if row.version == 2 && row.text == "authorized operator B edit"));
        assert_eq!(leader_request(&mut b, &original, "forged-A").await, accepted);
        let mut claimed_owner = request("owned", 2, Some("forged-A"));
        claimed_owner.action = QueueAction::Prioritize { new_text: Some("must not edit".into()) };
        assert!(matches!(leader_request(&mut b, &claimed_owner, "A").await.result,
            QueueResult::Rejected { reason: QueueRejection::NotOwner, .. }));
        let mut ungranted = delegated.clone();
        ungranted.action = QueueAction::Observe;
        assert!(matches!(submit_as(&sender, QueueCaller::for_test_session("other-session"), ungranted).await.result,
            QueueResult::Rejected { reason: QueueRejection::Unauthorized, row: None }));
        let state = actor.state.lock().await;
        let row = state.pending_inputs[0].queue_meta.as_ref().unwrap();
        assert_eq!(row.version, 2);
        assert_eq!(row.text, "authorized operator B edit");
        assert!(state.running_task.is_none());
        drop(state);
        a.cancel();
        b.cancel();
        server.cancel.cancel();
        agent
    }).await;
    drop(local);
    drop(keepalive);
}

#[tokio::test(flavor = "current_thread")]
async fn oversized_queue_protocol_reports_usage_error_before_session_lookup() {
    use acp::Agent as _;
    let local = tokio::task::LocalSet::new();
    let keepalive = local
        .run_until(async {
            let agent = crate::agent::mvp_agent::tests::build_minimal_agent_for_tests();
            let raw = format!(
                "{{\"sessionId\":\"absent\",\"padding\":\"{}\"}}",
                "x".repeat(crate::extensions::interject::MAX_QUEUE_REQUEST_BYTES)
            );
            let raw = serde_json::value::RawValue::from_string(raw).unwrap();
            let error = agent
                .ext_method(acp::ExtRequest::new("x.ai/queue/interject", raw.into()))
                .await
                .unwrap_err();
            assert_eq!(error.code, acp::Error::invalid_params().code);
            let diagnostic = error
                .data
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .unwrap();
            assert!(diagnostic.contains("67108864 protocol bytes"));
            assert!(diagnostic.contains("no operation admitted"));
            agent
        })
        .await;
    drop(local);
    drop(keepalive);
}
