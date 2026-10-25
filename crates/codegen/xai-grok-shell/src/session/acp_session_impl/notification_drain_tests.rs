use super::*;
#[path = "completion_suppression_tests.rs"]
mod suppression;
#[path = "notification_batch_payload_tests.rs"]
mod batch_payload;
use super::super::stop_gate::{TurnWaitDeclaration, WaitOnTarget};
use std::sync::Arc;
use tokio::sync::mpsc;
use xai_message_delivery_core::AttemptId;

async fn actor_with_channels() -> (SessionActor, mpsc::UnboundedReceiver<PersistenceMsg>) {
    let (gateway_tx, _gateway_rx) = mpsc::unbounded_channel();
    let (persistence_tx, persistence_rx) = mpsc::unbounded_channel();
    let actor =
        super::super::support::create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
    (actor, persistence_rx)
}

/// Answers the completion-receipt probe the promoter runs for a subagent-wake front.
fn spawn_completion_state_responder(mut persistence_rx: mpsc::UnboundedReceiver<PersistenceMsg>) {
    tokio::task::spawn_local(async move {
        while let Some(msg) = persistence_rx.recv().await {
            match msg {
                PersistenceMsg::GetCompletionState { respond_to } => {
                    let _ = respond_to.send(Ok(
                        crate::session::completion_delivery::CompletionDeliveryState::default(),
                    ));
                }
                _ => {}
            }
        }
    });
}

async fn arm_wake_barrier(actor: &SessionActor) {
    if let Some(gate) = &actor.tool_context.task_wake_suppressed {
        gate.set(true);
    }
    actor.state.lock().await.notifications_suppressed = true;
}

fn subagent_completed_row(
    prompt_id: &str,
    subagent_id: &str,
) -> (InputItem, oneshot::Receiver<PromptTurnResult>) {
    super::super::support::input_with_origin_rx(
        prompt_id,
        crate::session::PromptOrigin::SubagentCompleted {
            completion: crate::session::CompletionWake {
                id: xai_chat_state::SubagentCompletionId {
                    subagent_id: subagent_id.to_string(),
                    attempt_id: AttemptId::mint(7),
                },
                kind: crate::session::CompletionWakeKind::Fresh,
            },
        },
    )
}

async fn try_promote(actor: &Arc<SessionActor>) {
    let (completion_tx, _completion_rx) = mpsc::unbounded_channel();
    SessionActor::maybe_start_running_task(Arc::clone(actor), completion_tx).await;
}

#[tokio::test(flavor = "current_thread")]
async fn completion_wake_parks_under_barrier_and_promotes_a_caller_turn_on_clear() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, persistence_rx) = actor_with_channels().await;
            spawn_completion_state_responder(persistence_rx);
            arm_wake_barrier(&actor).await;
            let actor = Arc::new(actor);
            let (wake_item, _wake_rx) = subagent_completed_row("subagent-completed-parked", "sub-1");
            {
                let mut state = actor.state.lock().await;
                state.pending_inputs.push_back(wake_item);
            }

            // Parked: the wake stays queued and no caller turn starts while the barrier holds.
            try_promote(&actor).await;
            {
                let state = actor.state.lock().await;
                assert!(state.running_task.is_none(), "the wake must park, not run");
                assert_eq!(
                    state
                        .pending_inputs
                        .iter()
                        .map(|item| item.prompt_id.as_str())
                        .collect::<Vec<_>>(),
                    vec!["subagent-completed-parked"]
                );
            }

            let (respond_to, _response) = oneshot::channel();
            assert!(!actor.queue_input(QueueInputRequest::from_input_origin(
                Vec::new(),
                "human-resume".to_string(),
                InputOrigin::new(crate::session::PromptOrigin::User),
                PromptMode::Agent,
                respond_to,
            )).await);
            try_promote(&actor).await;
            let promoted = {
                let state = actor.state.lock().await;
                state.running_task.as_ref().map(|task| task.prompt_id.clone())
            };
            assert_eq!(
                promoted.as_deref(),
                Some("subagent-completed-parked"),
                "the parked wake must promote into a caller turn after the clear"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_queued_human_row_jumps_a_parked_completion_wake() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, persistence_rx) = actor_with_channels().await;
            spawn_completion_state_responder(persistence_rx);
            arm_wake_barrier(&actor).await;
            let actor = Arc::new(actor);
            let (wake_item, _wake_rx) = subagent_completed_row("subagent-completed-parked", "sub-2");
            let (human_item, _human_rx) = super::super::support::input_with_origin_rx(
                "user-next",
                crate::session::PromptOrigin::User,
            );
            {
                let mut state = actor.state.lock().await;
                state.pending_inputs.push_back(wake_item);
                state.pending_inputs.push_back(human_item);
            }

            try_promote(&actor).await;
            let (running, queued) = {
                let state = actor.state.lock().await;
                (
                    state.running_task.as_ref().map(|t| t.prompt_id.clone()),
                    state
                        .pending_inputs
                        .iter()
                        .map(|item| item.prompt_id.as_str())
                        .collect::<Vec<_>>(),
                )
            };
            assert_eq!(
                running.as_deref(),
                Some("user-next"),
                "human intent must jump the parked synthetic wake"
            );
            assert_eq!(
                queued,
                vec!["user-next", "subagent-completed-parked"],
                "the parked wake stays queued behind the human row"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn transient_notification_overflow_cannot_evict_parked_completions() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _persistence_rx) = actor_with_channels().await;
        arm_wake_barrier(&actor).await;
        let mut state = actor.state.lock().await;
        for index in 0..MAX_PENDING_NOTIFICATIONS + 5 {
            for source in [
                NotificationSource::BashTaskCompleted { task_id: format!("bash-{index}") },
                NotificationSource::MonitorCompleted { task_id: format!("monitor-{index}") },
                NotificationSource::MonitorEvent { task_id: format!("event-{index}") },
            ] {
                SessionActor::push_pending_notification(&mut state, PendingNotification {
                    prompt_id: format!("notification-{index}"),
                    prompt_blocks: Vec::new(),
                    priority: NotificationPriority::Later,
                    source,
                });
            }
        }
        assert_eq!(state.pending_notifications.iter().filter(|n| matches!(n.source, NotificationSource::BashTaskCompleted { .. })).count(), MAX_PENDING_NOTIFICATIONS + 5);
        assert_eq!(state.pending_notifications.iter().filter(|n| matches!(n.source, NotificationSource::MonitorCompleted { .. })).count(), MAX_PENDING_NOTIFICATIONS + 5);
        assert_eq!(state.pending_notifications.iter().filter(|n| matches!(n.source, NotificationSource::MonitorEvent { .. })).count(), MAX_PENDING_NOTIFICATIONS);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn state_only_suppression_parks_subagent_wakes() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _persistence_rx) = actor_with_channels().await;
        let (wake, _response) = subagent_completed_row("subagent-completed-state-only", "state-only");
        {
            let mut state = actor.state.lock().await;
            state.notifications_suppressed = true;
            state.pending_inputs.push_back(wake);
        }
        let actor = Arc::new(actor);
        try_promote(&actor).await;
        let state = actor.state.lock().await;
        assert!(state.running_task.is_none());
        assert_eq!(state.pending_inputs.len(), 1);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_waited_task_completion_wakes_even_under_the_active_goal_gate() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence_rx) = actor_with_channels().await;
            actor.goal_harness_enabled
                .store(true, std::sync::atomic::Ordering::Relaxed);
            actor.goal_tracker.lock().create_goal(
                "goal-wait".to_string(),
                "ship the thing".to_string(),
                None,
                0,
                chrono::Utc::now().to_rfc3339(),
                None,
            );
            actor.record_turn_wait(TurnWaitDeclaration::of([
                WaitOnTarget::BashTask {
                    task_id: "bash-wait-9".to_string(),
                },
            ]));
            {
                let mut state = actor.state.lock().await;
                for task_id in ["bash-wait-9", "bash-other"] {
                    SessionActor::push_pending_notification(
                        &mut state,
                        PendingNotification {
                            prompt_id: format!("notif-{task_id}"),
                            prompt_blocks: vec![acp::ContentBlock::Text(acp::TextContent::new(
                                format!("completion {task_id}"),
                            ))],
                            priority: NotificationPriority::Next,
                            source: NotificationSource::BashTaskCompleted {
                                task_id: task_id.to_string(),
                            },
                        },
                    );
                }
            }

            let actor = Arc::new(actor);
            let (completion_tx, _completion_rx) = mpsc::unbounded_channel();
            SessionActor::maybe_drain_notifications(Arc::clone(&actor), completion_tx).await;

            let (running, pending_notifications) = {
                let state = actor.state.lock().await;
                (
                    state.running_task.as_ref().map(|t| t.prompt_id.clone()),
                    state.pending_notifications.len(),
                )
            };
            assert!(
                running.as_deref().is_some_and(|id| id.starts_with("notifications-")),
                "the waited completion must wake the session into a drain turn, got {running:?}"
            );
            assert_eq!(
                pending_notifications, 0,
                "both completions are settled as reported (one surfaced, one goal-gate dropped)"
            );
        })
        .await;
}
