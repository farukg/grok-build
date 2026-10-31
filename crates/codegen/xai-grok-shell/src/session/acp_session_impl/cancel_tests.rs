use super::*;
use tokio::sync::mpsc;

async fn actor_with_channels() -> (SessionActor, mpsc::UnboundedReceiver<PersistenceMsg>) {
    let (gateway_tx, _gateway_rx) = mpsc::unbounded_channel();
    let (persistence_tx, mut requests) = mpsc::unbounded_channel();
    let (observations, persistence_rx) = mpsc::unbounded_channel();
    tokio::task::spawn_local(async move {
        while let Some(request) = requests.recv().await {
            let request = match request {
                PersistenceMsg::CompletionWakePolicy { policy, respond_to } => {
                    let _ = respond_to.send(Ok(()));
                    let (respond_to, _) = oneshot::channel();
                    PersistenceMsg::CompletionWakePolicy { policy, respond_to }
                }
                PersistenceMsg::SealHumanStoppedCompletions { ids, workflow_episodes, respond_to } => {
                    let _ = respond_to.send(Ok(()));
                    let (respond_to, _) = oneshot::channel();
                    PersistenceMsg::SealHumanStoppedCompletions { ids, workflow_episodes, respond_to }
                }
                other => other,
            };
            let _ = observations.send(request);
        }
    });
    let actor =
        super::super::support::create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
    (actor, persistence_rx)
}

fn create_active_goal(actor: &SessionActor) {
    actor.goal_tracker.lock().create_goal(
        "goal-1".to_string(),
        "ship the thing".to_string(),
        None,
        0,
        chrono::Utc::now().to_rfc3339(),
        None,
    );
}

fn wake_barrier_armed(actor: &SessionActor) -> bool {
    actor
        .tool_context
        .task_wake_suppressed
        .as_ref()
        .is_some_and(|gate| gate.get())
}

fn persisted_goal_status(
    persistence_rx: &mut mpsc::UnboundedReceiver<PersistenceMsg>,
) -> Vec<&'static str> {
    let mut statuses = Vec::new();
    while let Ok(msg) = persistence_rx.try_recv() {
        if let PersistenceMsg::GoalModeState(orchestration) = msg {
            statuses.push(match orchestration.status {
                crate::session::goal_tracker::GoalStatus::Active => "active",
                crate::session::goal_tracker::GoalStatus::UserPaused => "user_paused",
                crate::session::goal_tracker::GoalStatus::BackOffPaused => "back_off_paused",
                crate::session::goal_tracker::GoalStatus::NoProgressPaused => "no_progress_paused",
                crate::session::goal_tracker::GoalStatus::InfraPaused => "infra_paused",
                crate::session::goal_tracker::GoalStatus::Blocked => "blocked",
                crate::session::goal_tracker::GoalStatus::BudgetLimited => "budget_limited",
                crate::session::goal_tracker::GoalStatus::Complete => "complete",
            });
        }
    }
    statuses
}

#[tokio::test(flavor = "current_thread")]
async fn stop_waits_for_policy_ack_and_never_settles_a_failed_or_lost_commit() {
    enum Failure { Write, LostAck, ClosedOwner }
    tokio::task::LocalSet::new().run_until(async {
        for failure in [Failure::Write, Failure::LostAck, Failure::ClosedOwner] {
            let (gateway, _notifications) = mpsc::unbounded_channel();
            let (persistence, mut requests) = mpsc::unbounded_channel();
            let actor = Arc::new(super::super::support::create_test_actor(0, 256_000, 85, gateway, persistence).await);
            if matches!(failure, Failure::ClosedOwner) { requests.close(); }
            let cancel = tokio::task::spawn_local({
                let actor = actor.clone();
                async move {
                    actor.cancel_running_task(crate::session::CancelOptions {
                        trigger: Some(crate::session::CancelTrigger::CtrlC),
                        user_initiated: true, ..Default::default()
                    }).await
                }
            });
            match failure {
                Failure::Write | Failure::LostAck => {
                    let PersistenceMsg::CompletionWakePolicy { policy, respond_to } = requests.recv().await.unwrap() else {
                        panic!("Stop must first commit its policy");
                    };
                    assert_eq!(policy, crate::session::completion_delivery::CompletionWakePolicy::HumanStopped);
                    assert!(!cancel.is_finished(), "Stop cannot settle before its policy ACK");
                    assert!(wake_barrier_armed(&actor));
                    match failure {
                        Failure::Write => { respond_to.send(Err(std::io::Error::other("injected policy write failure"))).unwrap(); }
                        Failure::LostAck => drop(respond_to),
                        Failure::ClosedOwner => unreachable!(),
                    }
                }
                Failure::ClosedOwner => {}
            }
            let outcome = cancel.await.unwrap();
            assert!(!outcome.settled);
            assert!(matches!(outcome.policy, StopPolicyCommit::Failed(_)));
            assert_eq!(outcome.barrier, WakeBarrier::Armed);
            assert!(wake_barrier_armed(&actor));
            assert!(actor.state.lock().await.notifications_suppressed);
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn live_failed_stop_releases_exact_turn_before_subsequent_human_input() {
    enum Failure { Write, LostAck, ClosedOwner, RevocationClosed, RevocationLostAck, RevocationError, SuppressionWrite, SuppressionLostAck }
    tokio::task::LocalSet::new().run_until(async {
        for failure in [Failure::Write, Failure::LostAck, Failure::ClosedOwner, Failure::RevocationClosed, Failure::RevocationLostAck, Failure::RevocationError, Failure::SuppressionWrite, Failure::SuppressionLostAck] {
            let (gateway, _notifications) = mpsc::unbounded_channel();
            let (persistence, mut requests) = mpsc::unbounded_channel();
            let mut actor = super::super::support::create_test_actor(0, 256_000, 85, gateway, persistence).await;
            let (children, mut child_requests) = mpsc::unbounded_channel();
            if matches!(failure, Failure::RevocationClosed | Failure::RevocationLostAck | Failure::RevocationError) {
                actor.tool_context.subagent_event_tx = Some(children);
            }
            if matches!(failure, Failure::RevocationClosed) { child_requests.close(); }
            let actor = Arc::new(actor);
            let epoch = actor.turn_report.start_next_turn();
            let runner = tokio::task::spawn_local(std::future::pending::<()>());
            let identity = {
                let mut state = actor.state.lock().await;
                state.pending_inputs.push_back(super::super::support::user_item("failed-stop-live", "owner"));
                let task = AgentTask::new_at_epoch("failed-stop-live", epoch, runner.abort_handle());
                let identity = task.identity.clone();
                state.running_task = Some(task);
                identity
            };
            if matches!(failure, Failure::ClosedOwner) { requests.close(); }
            let cancel = tokio::task::spawn_local({
                let actor = actor.clone();
                async move {
                    actor.cancel_running_task(crate::session::CancelOptions {
                        trigger: Some(crate::session::CancelTrigger::CtrlC),
                        user_initiated: true, ..Default::default()
                    }).await
                }
            });
            match failure {
                Failure::Write | Failure::LostAck | Failure::RevocationClosed | Failure::RevocationLostAck
                | Failure::RevocationError | Failure::SuppressionWrite | Failure::SuppressionLostAck => {
                    let PersistenceMsg::CompletionWakePolicy { policy, respond_to } = requests.recv().await.unwrap() else {
                        panic!("Stop must first commit its policy");
                    };
                    assert_eq!(policy, crate::session::completion_delivery::CompletionWakePolicy::HumanStopped);
                    assert!(!cancel.is_finished(), "Stop cannot settle before its policy ACK");
                    assert!(wake_barrier_armed(&actor));
                    match failure {
                        Failure::Write => { respond_to.send(Err(std::io::Error::other("injected policy write failure"))).unwrap(); }
                        Failure::LostAck => drop(respond_to),
                        Failure::RevocationClosed | Failure::RevocationLostAck | Failure::RevocationError
                        | Failure::SuppressionWrite | Failure::SuppressionLostAck => { respond_to.send(Ok(())).unwrap(); }
                        Failure::ClosedOwner => unreachable!(),
                    }
                }
                Failure::ClosedOwner => {}
            }
            if matches!(failure, Failure::RevocationLostAck) {
                drop(child_requests.recv().await.unwrap());
            }
            if matches!(failure, Failure::RevocationError) {
                let xai_grok_tools::implementations::grok_build::task::types::SubagentEvent::RevokeParentSession { respond_to, .. } = child_requests.recv().await.unwrap() else {
                    panic!("Stop must request parent revocation");
                };
                respond_to.send(Err(xai_grok_tools::implementations::grok_build::task::types::SubagentAccessError::ParentSessionUnavailable)).unwrap();
            }
            if matches!(failure, Failure::SuppressionWrite | Failure::SuppressionLostAck) {
                let PersistenceMsg::SealHumanStoppedCompletions { respond_to, .. } = requests.recv().await.unwrap() else {
                    panic!("Stop must seal registered identities");
                };
                assert!(!cancel.is_finished(), "Stop cannot settle before its suppression ACK");
                assert!(wake_barrier_armed(&actor));
                if matches!(failure, Failure::SuppressionWrite) {
                    respond_to.send(Err(std::io::Error::other("injected suppression write failure"))).unwrap();
                } else {
                    drop(respond_to);
                }
            }
            let persistence_drain = tokio::task::spawn_local(async move {
                while let Some(request) = requests.recv().await {
                    match request {
                        PersistenceMsg::FlushAndAck { respond_to }
                        | PersistenceMsg::CompletionWakePolicy { respond_to, .. }
                        | PersistenceMsg::SealHumanStoppedCompletions { respond_to, .. } => {
                            let _ = respond_to.send(Ok(()));
                        }
                        PersistenceMsg::AppendUpdateDurablyAndAck { respond_to, .. } => {
                            let _ = respond_to.send(Ok(()));
                        }
                        _ => {}
                    }
                }
            });
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), cancel).await.unwrap().unwrap();
            assert!(!outcome.settled);
            let StopPolicyCommit::Failed(error) = outcome.policy else { panic!("failed Stop must retain its error") };
            let expected = match failure {
                Failure::Write => "injected policy write failure",
                Failure::LostAck => "Stop policy commit acknowledgement lost",
                Failure::ClosedOwner => "Stop policy persistence owner closed",
                Failure::RevocationClosed => "Stop child revocation owner closed: Subagent request not confirmed: coordinator unavailable or response lost",
                Failure::RevocationLostAck => "Stop child revocation acknowledgement lost",
                Failure::RevocationError => "Parent session is not bound to this backend",
                Failure::SuppressionWrite => "injected suppression write failure",
                Failure::SuppressionLostAck => "Stop suppression commit acknowledgement lost",
            };
            assert_eq!(error.to_string(), expected);
            assert_eq!(outcome.barrier, WakeBarrier::Armed);
            assert!(wake_barrier_armed(&actor));
            assert!(actor.state.lock().await.notifications_suppressed);
            assert!(runner.await.unwrap_err().is_cancelled());
            {
                let state = actor.state.lock().await;
                assert!(state.running_task.is_none());
                assert!(!state.finalization_gate.is_active());
            }
            assert!(!actor.handle_completion(
                "failed-stop-live".into(), epoch, &identity,
                Ok(PromptTurnOk {
                    stop_reason: acp::StopReason::EndTurn, total_tokens: 0, turn_snapshot: None,
                    completion_kind: PromptCompletionKind::Completed, structured_output: None,
                    usage: None, tool_overrides: None,
                }), None,
            ).await);
            let (respond_to, _response) = oneshot::channel();
            assert!(!actor.queue_input(QueueInputRequest::from_legacy_prompt_id(
                vec![acp::ContentBlock::Text(acp::TextContent::new("new human input"))],
                "after-failed-stop".into(), PromptMode::Agent, respond_to,
            )).await);
            let (completion_tx, _completion_rx) = mpsc::unbounded_channel();
            actor.clone().maybe_start_running_task(completion_tx).await;
            let state = actor.state.lock().await;
            assert_eq!(state.running_prompt_id(), Some("after-failed-stop"));
            assert!(wake_barrier_armed(&actor), "promotion is not Stop authority");
            state.running_task.as_ref().unwrap().abort();
            drop(state);
            persistence_drain.abort();
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn stop_gesture_pauses_active_goal_and_arms_the_wake_barrier() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, mut persistence_rx) = actor_with_channels().await;
            create_active_goal(&actor);
            assert_eq!(
                actor.goal_tracker.lock().status(),
                Some(crate::session::goal_tracker::GoalStatus::Active)
            );

            let outcome = actor
                .cancel_running_task(crate::session::CancelOptions {
                    trigger: Some(crate::session::CancelTrigger::CtrlC),
                    user_initiated: true,
                    ..Default::default()
                })
                .await;

            assert!(matches!(outcome.barrier, WakeBarrier::Armed));
            assert!(wake_barrier_armed(&actor));
            assert!(actor.state.lock().await.notifications_suppressed);
            // gb-goalstate-9: the stopped session's goal status snapshot reads Paused.
            assert_eq!(
                actor.goal_tracker.lock().status(),
                Some(crate::session::goal_tracker::GoalStatus::UserPaused)
            );
            assert_eq!(
                persisted_goal_status(&mut persistence_rx),
                vec!["user_paused"],
                "the pause must publish a user_paused status snapshot"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn send_now_cancel_leaves_the_goal_active() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence_rx) = actor_with_channels().await;
            create_active_goal(&actor);

            let outcome = actor
                .cancel_running_task(crate::session::CancelOptions {
                    trigger: Some(crate::session::CancelTrigger::SendNow),
                    ..Default::default()
                })
                .await;

            assert!(matches!(outcome.barrier, WakeBarrier::Clear));
            assert!(!wake_barrier_armed(&actor));
            assert_eq!(
                actor.goal_tracker.lock().status(),
                Some(crate::session::goal_tracker::GoalStatus::Active),
                "a send-now redirect is the user continuing; it must not pause the goal"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn stop_gesture_without_goal_only_arms_the_barrier() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence_rx) = actor_with_channels().await;
            let outcome = actor
                .cancel_running_task(crate::session::CancelOptions {
                    trigger: Some(crate::session::CancelTrigger::Esc),
                    user_initiated: true,
                    ..Default::default()
                })
                .await;
            assert!(matches!(outcome.barrier, WakeBarrier::Armed));
            assert!(wake_barrier_armed(&actor));
            assert_eq!(actor.goal_tracker.lock().status(), None);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn idle_stop_removes_old_queued_work_without_faking_a_producer_terminal() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (actor, _persistence_rx) = actor_with_channels().await;
            let (wake, mut receipt) = super::super::support::input_with_origin_rx(
                "workflow-completed-parked-0",
                crate::session::PromptOrigin::WorkflowCompleted {
                    completion_id: "parked-0".to_string(),
                },
            );
            actor.state.lock().await.pending_inputs.push_back(wake);
            let outcome = actor
                .cancel_running_task(crate::session::CancelOptions {
                    trigger: Some(crate::session::CancelTrigger::CtrlC),
                    user_initiated: true,
                    ..Default::default()
                })
                .await;
            assert_eq!(outcome.barrier, WakeBarrier::Armed);
            let state = actor.state.lock().await;
            assert!(state.running_task.is_none());
            assert!(state.pending_inputs.is_empty());
            let removed = receipt.try_recv().unwrap().unwrap();
            assert!(matches!(removed.completion_kind, PromptCompletionKind::RemovedFromQueue));
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn generic_resume_preserves_explicit_stop() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, mut persistence_rx) = actor_with_channels().await;
            let outcome = actor
                .cancel_running_task(crate::session::CancelOptions {
                    trigger: Some(crate::session::CancelTrigger::CtrlC),
                    user_initiated: true,
                    ..Default::default()
                })
                .await;
            assert_eq!(outcome.barrier, WakeBarrier::Armed);
            assert!(outcome.settled);
            assert!(!outcome.turn_stopped);
            assert!(wake_barrier_armed(&actor));

            for authority in [WakeReengagement::GoalResume, WakeReengagement::SendNow, WakeReengagement::Rewind] {
                actor.clear_wake_suppression(authority).await.unwrap();
                assert!(wake_barrier_armed(&actor));
                assert!(actor.state.lock().await.notifications_suppressed);
            }

            assert!(wake_barrier_armed(&actor));
            assert!(actor.state.lock().await.notifications_suppressed);
            while let Ok(msg) = persistence_rx.try_recv() {
                assert!(!matches!(msg, PersistenceMsg::CompletionWakePolicy {
                    policy: crate::session::completion_delivery::CompletionWakePolicy::Automatic, ..
                }));
            }
        })
        .await;
}
