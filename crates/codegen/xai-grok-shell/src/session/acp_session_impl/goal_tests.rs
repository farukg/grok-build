use super::*;
use tokio::sync::mpsc;
use xai_workflow::PhaseMeta;

async fn actor_with_channels() -> (SessionActor, mpsc::UnboundedReceiver<PersistenceMsg>) {
    let (gateway_tx, _gateway_rx) = mpsc::unbounded_channel();
    let (persistence_tx, persistence_rx) = mpsc::unbounded_channel();
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

async fn seed_active_workflow_run(actor: &SessionActor, run_id: &str) {
    actor
        .workflow_tracker()
        .await
        .lock()
        .start_run(
            run_id.to_string(),
            "probe run".to_string(),
            "objective".to_string(),
            vec![PhaseMeta {
                title: "phase".to_string(),
                detail: None,
            }],
            None,
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

#[tokio::test(flavor = "current_thread")]
async fn resume_goal_clears_the_suppression_barrier_in_the_resume_transaction() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence_rx) = actor_with_channels().await;
            create_active_goal(&actor);
            assert!(
                actor
                    .goal_tracker
                    .lock()
                    .pause(crate::session::goal_tracker::GoalPauseReason::User)
            );
            // Arm the post-stop barrier the way a StopGesture leaves it.
            actor.set_completion_wake_policy(
                crate::session::completion_delivery::CompletionWakePolicy::HumanStopped,
            );
            actor.state.lock().await.notifications_suppressed = true;
            seed_active_workflow_run(&actor, "wf-resume-1").await;
            actor.record_turn_wait(actor.snapshot_turn_wait().await);
            assert!(wake_barrier_armed(&actor));

            let outcome = actor.resume_goal().await;
            let GoalResumeOutcome::Inference { user_msg, .. } = outcome else {
                panic!("a paused goal must resume into inference")
            };
            assert_eq!(user_msg, "Goal resumed.");
            assert!(
                !wake_barrier_armed(&actor),
                "the resume transaction must clear the wake barrier"
            );
            assert!(
                !actor.state.lock().await.notifications_suppressed,
                "the resume transaction must clear notification suppression"
            );
            assert_eq!(
                actor.goal_tracker.lock().status(),
                Some(crate::session::goal_tracker::GoalStatus::Active)
            );
            assert!(
                !actor.declared_wait_blocks_continuation().await,
                "resume is new human input and must break a declared wait"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_goal_continuation_suppressed_while_declared_wait_is_live() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence_rx) = actor_with_channels().await;
            actor
                .goal_harness_enabled
                .store(true, std::sync::atomic::Ordering::Relaxed);
            create_active_goal(&actor);

            let plan = actor
                .prepare_goal_continuation(0, GoalContinuationPurpose::TurnEnd)
                .await;
            assert!(
                plan.is_some(),
                "an active goal with no declared wait must plan a continuation"
            );

            seed_active_workflow_run(&actor, "wf-continuation-1").await;
            actor.record_turn_wait(actor.snapshot_turn_wait().await);
            let plan = actor
                .prepare_goal_continuation(0, GoalContinuationPurpose::TurnEnd)
                .await;
            assert!(
                plan.is_none(),
                "a live declared wait must suppress the continuation re-wake"
            );

            actor.break_declared_wait();
            let plan = actor
                .prepare_goal_continuation(0, GoalContinuationPurpose::TurnEnd)
                .await;
            assert!(
                plan.is_some(),
                "breaking the wait must let the goal loop continue again"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn goal_evaluator_accepts_stop_gate_wait_without_pausing_or_nudging() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _persistence_rx) = actor_with_channels().await;
        actor.goal_harness_enabled.store(true, std::sync::atomic::Ordering::Relaxed);
        create_active_goal(&actor);
        seed_active_workflow_run(&actor, "live-work").await;
        let mut calls = vec![xai_grok_sampling_types::conversation::ToolCall {
            id: "declare-wait".into(), name: super::super::stop_gate::DECLARE_WAIT_TOOL.to_string(),
            arguments: r#"{"targets":[{"kind":"workflow_run","run_id":"live-work"}]}"#.into(),
        }];
        actor.chat_state_handle.push_assistant_response(ConversationItem::assistant_tool_calls(calls.clone()));
        assert!(matches!(actor.handle_wait_tool_call(&mut calls).await, super::super::stop_gate::WaitToolStep::Declared));
        assert!(matches!(actor.run_stop_gate("waiting", 0).await, StopGateDecision::AllowStop { .. }));
        assert!(matches!(actor.run_goal_round_end().await, GoalRoundDecision::EndTurn));
        assert_eq!(actor.goal_tracker.lock().status(), Some(crate::session::goal_tracker::GoalStatus::Active));
        assert!(actor.state.lock().await.pending_inputs.is_empty());
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_goal_continuation_requires_an_active_goal_even_with_live_wait_targets() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence_rx) = actor_with_channels().await;
            actor
                .goal_harness_enabled
                .store(true, std::sync::atomic::Ordering::Relaxed);
            create_active_goal(&actor);
            assert!(
                actor
                    .goal_tracker
                    .lock()
                    .pause(crate::session::goal_tracker::GoalPauseReason::User)
            );
            seed_active_workflow_run(&actor, "wf-paused-1").await;
            actor.record_turn_wait(actor.snapshot_turn_wait().await);

            let plan = actor
                .prepare_goal_continuation(0, GoalContinuationPurpose::TurnEnd)
                .await;
            assert!(
                plan.is_none(),
                "a paused goal must not continue, wait or not"
            );
        })
        .await;
}
