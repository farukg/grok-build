use super::*;
use std::sync::Arc;
use tokio::sync::mpsc;
use xai_workflow::PhaseMeta;

async fn actor_with_channels() -> (SessionActor, mpsc::UnboundedReceiver<PersistenceMsg>) {
    let (gateway_tx, _gateway_rx) = mpsc::unbounded_channel();
    let (persistence_tx, persistence_rx) = mpsc::unbounded_channel();
    let actor =
        super::super::support::create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
    (actor, persistence_rx)
}

/// Start a run and drive it to a terminal, unreported completion.
async fn seed_unreported_workflow_run(actor: &SessionActor, run_id: &str) {
    let tracker = actor.workflow_tracker().await;
    let mut tracker = tracker.lock();
    tracker.start_run(
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
    tracker.interrupt(run_id, "engine stopped");
}

async fn arm_wake_barrier(actor: &SessionActor) {
    if let Some(gate) = &actor.tool_context.task_wake_suppressed {
        gate.set(true);
    }
    actor.state.lock().await.notifications_suppressed = true;
}

async fn try_promote(actor: &Arc<SessionActor>) {
    let (completion_tx, _completion_rx) = mpsc::unbounded_channel();
    SessionActor::maybe_start_running_task(Arc::clone(actor), completion_tx).await;
}

fn queued_prompt_ids(state: &State) -> Vec<&str> {
    state
        .pending_inputs
        .iter()
        .map(|item| item.prompt_id.as_str())
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn workflow_completion_wake_parks_under_suppression_instead_of_dropping() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence_rx) = actor_with_channels().await;
            arm_wake_barrier(&actor).await;
            seed_unreported_workflow_run(&actor, "run1").await;
            let actor = Arc::new(actor);
            let (completion_tx, _completion_rx) = mpsc::unbounded_channel();

            actor
                .clone()
                .handle_workflow_completion_turn(completion_tx.clone(), "run1".to_string(), 0)
                .await;
            // A duplicate wake for the same run must not queue a second row.
            actor
                .clone()
                .handle_workflow_completion_turn(completion_tx, "run1".to_string(), 0)
                .await;
            {
                let state = actor.state.lock().await;
                assert_eq!(
                    queued_prompt_ids(&state),
                    vec!["workflow-completed-run1-0"],
                    "the suppressed wake must park in the queue exactly once, never drop"
                );
                assert!(
                    state.running_task.is_none(),
                    "no caller turn may start while the barrier is armed"
                );
            }

            let (respond_to, _response) = oneshot::channel();
            let request = QueueInputRequest::from_input_origin(
                Vec::new(),
                "human-resume".to_string(),
                InputOrigin::new(super::super::PromptOrigin::User),
                PromptMode::Agent,
                respond_to,
            );
            assert!(!actor.queue_input(request).await);
            try_promote(&actor).await;
            let promoted = {
                let state = actor.state.lock().await;
                state.running_task.as_ref().map(|task| task.prompt_id.clone())
            };
            assert_eq!(
                promoted.as_deref(),
                Some("workflow-completed-run1-0"),
                "the parked workflow wake must promote after the clear"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn workflow_completion_wake_delivers_when_unsuppressed() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence_rx) = actor_with_channels().await;
            seed_unreported_workflow_run(&actor, "run2").await;
            let actor = Arc::new(actor);
            let (completion_tx, _completion_rx) = mpsc::unbounded_channel();

            actor.handle_workflow_completion_turn(completion_tx, "run2".to_string(), 0).await;
            let running = {
                let state = actor.state.lock().await;
                state.running_task.as_ref().map(|task| task.prompt_id.clone())
            };
            assert_eq!(
                running.as_deref(),
                Some("workflow-completed-run2-0"),
                "an unsuppressed unreported completion must deliver its wake immediately"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn distinct_parked_workflow_completions_each_keep_their_wake() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _persistence_rx) = actor_with_channels().await;
        arm_wake_barrier(&actor).await;
        seed_unreported_workflow_run(&actor, "first").await;
        seed_unreported_workflow_run(&actor, "second").await;
        let actor = Arc::new(actor);
        let (completion_tx, _completion_rx) = mpsc::unbounded_channel();
        for run in ["first", "second", "first"] {
            actor.clone().handle_workflow_completion_turn(completion_tx.clone(), run.to_string(), 0).await;
        }
        let state = actor.state.lock().await;
        assert_eq!(queued_prompt_ids(&state), ["workflow-completed-first-0", "workflow-completed-second-0"]);
        assert!(state.running_task.is_none());
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn completed_workflow_wakes_caller_while_declared_peer_remains_live() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _persistence_rx) = actor_with_channels().await;
        for run in ["finished", "peer"] {
            actor.workflow_tracker().await.lock().start_run(
                run.to_string(), "probe".to_string(), "objective".to_string(),
                vec![PhaseMeta { title: "phase".to_string(), detail: None }], None, None,
            );
        }
        let mut calls = vec![xai_grok_sampling_types::conversation::ToolCall {
            id: "declare-wait".into(), name: super::super::stop_gate::DECLARE_WAIT_TOOL.to_string(),
            arguments: r#"{"targets":[{"kind":"workflow_run","run_id":"finished"},{"kind":"workflow_run","run_id":"peer"}]}"#.into(),
        }];
        actor.chat_state_handle.push_assistant_response(ConversationItem::assistant_tool_calls(calls.clone()));
        assert!(matches!(actor.handle_wait_tool_call(&mut calls).await, super::super::stop_gate::WaitToolStep::Declared));
        assert!(matches!(actor.run_stop_gate("waiting", 0).await, StopGateDecision::AllowStop { .. }));
        assert!(actor.declared_wait_blocks_continuation().await);
        actor.workflow_tracker().await.lock().interrupt("finished", "done");
        let actor = Arc::new(actor);
        let (completion_tx, _completion_rx) = mpsc::unbounded_channel();
        actor.clone().handle_workflow_completion_turn(completion_tx, "finished".to_string(), 0).await;
        assert_eq!(actor.state.lock().await.running_prompt_id(), Some("workflow-completed-finished-0"));
        assert!(!actor.declared_wait_blocks_continuation().await);
        assert!(!actor.snapshot_turn_wait().await.is_empty());
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn reported_workflow_completions_queue_no_wake() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence_rx) = actor_with_channels().await;
            seed_unreported_workflow_run(&actor, "run3").await;
            actor
                .workflow_tracker()
                .await
                .lock()
                .mark_completion_reported("run3");
            let actor = Arc::new(actor);
            let (completion_tx, _completion_rx) = mpsc::unbounded_channel();

            actor
                .handle_workflow_completion_turn(completion_tx, "run3".to_string(), 0)
                .await;
            let state = actor.state.lock().await;
            assert!(
                state.pending_inputs.is_empty(),
                "a reported completion has its receipt; it must not queue a wake"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn restore_completion_wake_policy_does_not_rearm_the_barrier() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, mut persistence_rx) = actor_with_channels().await;
            arm_wake_barrier(&actor).await;

            actor
                .restore_completion_wake_policy(
                    crate::session::completion_delivery::CompletionWakePolicy::HumanStopped,
                )
                .await;

            assert!(
                !actor
                    .tool_context
                    .task_wake_suppressed
                    .as_ref()
                    .is_some_and(|gate| gate.get()),
                "a persisted HumanStopped barrier must not survive the restart boundary"
            );
            assert!(
                !actor.state.lock().await.notifications_suppressed,
                "restored suppression flag must clear with the barrier"
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
                "the restore must persist Automatic so the next restart also starts clear"
            );
        })
        .await;
}
