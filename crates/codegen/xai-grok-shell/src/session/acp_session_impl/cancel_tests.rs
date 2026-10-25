use super::*;
use tokio::sync::mpsc;

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
async fn idle_stop_preserves_queued_workflow_obligation_without_a_fallback() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _persistence_rx) = actor_with_channels().await;
        let (wake, mut receipt) = super::super::support::input_with_origin_rx(
            "workflow-completed-parked-0",
            crate::session::PromptOrigin::WorkflowCompleted { completion_id: "parked-0".to_string() },
        );
        actor.state.lock().await.pending_inputs.push_back(wake);
        let outcome = actor.cancel_running_task(crate::session::CancelOptions {
            trigger: Some(crate::session::CancelTrigger::CtrlC),
            user_initiated: true,
            ..Default::default()
        }).await;
        assert_eq!(outcome.barrier, WakeBarrier::Armed);
        let state = actor.state.lock().await;
        assert!(state.running_task.is_none());
        assert_eq!(state.pending_inputs.len(), 1);
        assert_eq!(state.pending_inputs.front().unwrap().prompt_id, "workflow-completed-parked-0");
        assert!(matches!(receipt.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn clear_wake_suppression_re_admits_wakes_and_persists_automatic() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, mut persistence_rx) = actor_with_channels().await;
            actor
                .cancel_running_task(crate::session::CancelOptions {
                    trigger: Some(crate::session::CancelTrigger::CtrlC),
                    user_initiated: true,
                    ..Default::default()
                })
                .await;
            assert!(wake_barrier_armed(&actor));

            actor.clear_wake_suppression("goal_resume").await;

            assert!(!wake_barrier_armed(&actor), "the live gate must clear");
            assert!(
                !actor.state.lock().await.notifications_suppressed,
                "notification suppression must clear"
            );
            let mut saw_automatic = false;
            while let Ok(msg) = persistence_rx.try_recv() {
                if let PersistenceMsg::CompletionWakePolicy(
                    crate::session::completion_delivery::CompletionWakePolicy::Automatic,
                ) = msg
                {
                    saw_automatic = true;
                }
            }
            assert!(
                saw_automatic,
                "the clear must persist the Automatic policy so the barrier cannot survive a restart"
            );
        })
        .await;
}
