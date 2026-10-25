use super::*;
use std::sync::Arc;
use tokio::sync::mpsc;
use xai_grok_tools::implementations::grok_build::task::types::{
    ActiveSubagentSummary, SubagentEvent, SubagentListActiveRequest,
};
use xai_workflow::PhaseMeta;

async fn actor_with_channels() -> (
    SessionActor,
    mpsc::UnboundedReceiver<xai_acp_lib::AcpClientMessage>,
    mpsc::UnboundedReceiver<PersistenceMsg>,
) {
    let (gateway_tx, gateway_rx) = mpsc::unbounded_channel();
    let (persistence_tx, persistence_rx) = mpsc::unbounded_channel();
    let actor =
        super::super::support::create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
    (actor, gateway_rx, persistence_rx)
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

async fn declare_workflow_wait(actor: &SessionActor, run_ids: &[&str]) {
    let mut calls = vec![xai_grok_sampling_types::conversation::ToolCall {
        id: "declare-wait".into(),
        name: DECLARE_WAIT_TOOL.to_string(),
        arguments: serde_json::json!({ "targets": run_ids.iter().map(|id|
            serde_json::json!({ "kind": "workflow_run", "run_id": id })
        ).collect::<Vec<_>>() }).to_string().into(),
    }];
    actor.chat_state_handle.push_assistant_response(ConversationItem::assistant_tool_calls(calls.clone()));
    assert!(matches!(actor.handle_wait_tool_call(&mut calls).await, WaitToolStep::Declared));
}

#[tokio::test(flavor = "current_thread")]
async fn stop_gate_without_hooks_carries_the_explicit_wait_set() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
            seed_active_workflow_run(&actor, "wf-live-1").await;
            actor.set_goal_loop_active_resource(true).await;
            declare_workflow_wait(&actor, &["wf-live-1"]).await;

            let decision = actor.run_stop_gate("prompt-1", 0).await;
            assert!(!actor.tool_context.goal_loop_active_gate.load(std::sync::atomic::Ordering::Relaxed),
                "the waiting parent must let the task/monitor bridge submit completion wakes");
            let StopGateDecision::AllowStop { wait } = decision else {
                panic!("a hookless gate must allow the stop, got {decision:?}")
            };
            assert!(
                wait.contains(&WaitOnTarget::WorkflowRun {
                    run_id: "wf-live-1".to_string()
                }),
                "the declared wait must carry the live workflow run, got {wait:?}"
            );
            assert!(
                actor.declared_wait_blocks_continuation().await,
                "the recorded wait must block the goal continuation while the run is live"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn wait_blocks_until_target_finishes_or_user_input_breaks_it() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
            assert!(
                !actor.declared_wait_blocks_continuation().await,
                "no declared wait must not block"
            );

            seed_active_workflow_run(&actor, "wf-wait-1").await;
            actor.record_turn_wait(actor.snapshot_turn_wait().await);
            assert!(actor.declared_wait_blocks_continuation().await);

            // The run reaching a terminal state releases the wait without any local transition.
            actor
                .workflow_tracker()
                .await
                .lock()
                .interrupt("wf-wait-1", "session ended");
            assert!(
                !actor.declared_wait_blocks_continuation().await,
                "a finished target must release the wait"
            );

            // A fresh live declaration is broken by new user input.
            seed_active_workflow_run(&actor, "wf-wait-2").await;
            actor.record_turn_wait(actor.snapshot_turn_wait().await);
            assert!(actor.declared_wait_blocks_continuation().await);
            actor.break_declared_wait();
            assert!(
                !actor.declared_wait_blocks_continuation().await,
                "new user input must break the declared wait"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn subagent_wait_target_resolves_liveness_through_the_coordinator() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
            let subagent_listed = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SubagentEvent>();
            actor.tool_context.subagent_event_tx = Some(event_tx);
            let listed = Arc::clone(&subagent_listed);
            tokio::task::spawn_local(async move {
                while let Some(event) = event_rx.recv().await {
                    if let SubagentEvent::ListActive(SubagentListActiveRequest {
                        respond_to,
                        ..
                    }) = event
                    {
                        let summaries = if listed.load(std::sync::atomic::Ordering::Relaxed) {
                            vec![ActiveSubagentSummary {
                                subagent_id: "sub-wait-1".to_string(),
                                subagent_type: "explore".to_string(),
                                description: "probe".to_string(),
                                elapsed_ms: 10,
                            }]
                        } else {
                            Vec::new()
                        };
                        let _ = respond_to.send(summaries);
                    }
                }
            });

            let wait = actor.snapshot_turn_wait().await;
            assert!(
                wait.contains(&WaitOnTarget::Subagent {
                    subagent_id: "sub-wait-1".to_string()
                }),
                "a running child must appear in the declared wait, got {wait:?}"
            );
            let mut calls = vec![xai_grok_sampling_types::conversation::ToolCall {
                id: "declare-subagent".into(), name: DECLARE_WAIT_TOOL.to_string(),
                arguments: r#"{"targets":[{"kind":"subagent","subagent_id":"sub-wait-1"}]}"#.into(),
            }];
            actor.chat_state_handle.push_assistant_response(ConversationItem::assistant_tool_calls(calls.clone()));
            assert!(matches!(actor.handle_wait_tool_call(&mut calls).await, WaitToolStep::Declared));
            assert!(actor.declared_wait_blocks_continuation().await);

            subagent_listed.store(false, std::sync::atomic::Ordering::Relaxed);
            assert!(
                !actor.declared_wait_blocks_continuation().await,
                "a subagent that finished must release the wait"
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn absent_task_and_monitor_targets_never_block() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
            for target in [
                WaitOnTarget::BashTask {
                    task_id: "bash-absent".to_string(),
                },
                WaitOnTarget::Monitor {
                    task_id: "monitor-absent".to_string(),
                },
                WaitOnTarget::Subagent {
                    subagent_id: "sub-absent".to_string(),
                },
                WaitOnTarget::WorkflowRun {
                    run_id: "wf-absent".to_string(),
                },
            ] {
                actor.record_turn_wait(TurnWaitDeclaration::of([target.clone()]));
                assert!(
                    !actor.declared_wait_blocks_continuation().await,
                    "an absent target must never block: {target:?}"
                );
            }
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn any_completed_target_releases_wait_with_a_live_peer() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
        seed_active_workflow_run(&actor, "first").await;
        seed_active_workflow_run(&actor, "peer").await;
        declare_workflow_wait(&actor, &["first", "peer"]).await;
        assert!(matches!(actor.run_stop_gate("waiting", 0).await, StopGateDecision::AllowStop { .. }));
        assert!(actor.declared_wait_blocks_continuation().await);
        actor.workflow_tracker().await.lock().interrupt("first", "finished");
        assert!(!actor.declared_wait_blocks_continuation().await);
        assert!(actor.snapshot_turn_wait().await.contains(&WaitOnTarget::WorkflowRun {
            run_id: "peer".to_string(),
        }));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn accepted_human_input_breaks_wait_without_manual_state_transitions() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
        seed_active_workflow_run(&actor, "still-live").await;
        declare_workflow_wait(&actor, &["still-live"]).await;
        assert!(matches!(actor.run_stop_gate("waiting", 0).await, StopGateDecision::AllowStop { .. }));
        assert!(actor.declared_wait_blocks_continuation().await);
        let (respond_to, _response) = tokio::sync::oneshot::channel();
        assert!(!actor.queue_input(QueueInputRequest::from_input_origin(
            Vec::new(),
            "new-user-input".to_string(),
            InputOrigin::new(crate::session::PromptOrigin::User),
            PromptMode::Agent,
            respond_to,
        )).await);
        assert!(!actor.declared_wait_blocks_continuation().await);
        assert!(!actor.snapshot_turn_wait().await.is_empty());
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn undeclared_live_work_does_not_create_a_wait() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
        actor.goal_harness_enabled.store(true, std::sync::atomic::Ordering::Relaxed);
        actor.goal_tracker.lock().create_goal("goal".to_string(), "continue".to_string(), None, 0, chrono::Utc::now().to_rfc3339(), None);
        seed_active_workflow_run(&actor, "live-but-not-selected").await;
        let StopGateDecision::AllowStop { wait } = actor.run_stop_gate("finished", 0).await else {
            panic!("hookless stop must be allowed");
        };
        assert!(wait.is_empty());
        assert!(!actor.declared_wait_blocks_continuation().await);
        assert!(actor.prepare_goal_continuation(0, GoalContinuationPurpose::TurnEnd).await.is_some());
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_subset_ignores_unrelated_completion_and_accepts_selected_completion() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
        actor.goal_harness_enabled.store(true, std::sync::atomic::Ordering::Relaxed);
        actor.goal_tracker.lock().create_goal("goal".to_string(), "wait selectively".to_string(), None, 0, chrono::Utc::now().to_rfc3339(), None);
        seed_active_workflow_run(&actor, "selected").await;
        seed_active_workflow_run(&actor, "unrelated").await;
        declare_workflow_wait(&actor, &["selected"]).await;
        assert!(actor.prepare_goal_continuation(0, GoalContinuationPurpose::TurnEnd).await.is_none());
        let StopGateDecision::AllowStop { wait } = actor.run_stop_gate("finished", 0).await else {
            panic!("hookless stop must be allowed");
        };
        assert_eq!(wait, TurnWaitDeclaration::of([WaitOnTarget::WorkflowRun { run_id: "selected".to_string() }]));
        actor.workflow_tracker().await.lock().interrupt("unrelated", "done");
        assert!(actor.declared_wait_blocks_continuation().await, "an unselected registry-only completion must not satisfy the selected wait");
        for run_id in ["unrelated", "selected"] {
            if run_id == "selected" {
                declare_workflow_wait(&actor, &["selected"]).await;
            }
            let (respond_to, _response) = tokio::sync::oneshot::channel();
            assert!(!actor.queue_input(QueueInputRequest::from_input_origin(
                Vec::new(), format!("workflow-completed-{run_id}-0"),
                InputOrigin::new(crate::session::PromptOrigin::WorkflowCompleted { completion_id: format!("{run_id}-0") }),
                PromptMode::Agent, respond_to,
            )).await);
            assert!(!actor.declared_wait_blocks_continuation().await, "every admitted completion input breaks the wait");
            assert!(!actor.snapshot_turn_wait().await.is_empty(), "selected work remains live; admitted input, not an empty registry, cleared the declaration");
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn every_conversational_input_breaks_the_explicit_wait() {
    tokio::task::LocalSet::new().run_until(async {
        for origin in [
            PromptOrigin::User,
            PromptOrigin::Delivery { delivery_id: "delivery".to_string(), payload_digest: "digest".to_string() },
            PromptOrigin::ParentAgentMessage { message_id: "agent".to_string(), sender_session_id: "parent".to_string() },
            PromptOrigin::ParentHumanMessage { message_id: "human".to_string(), sender_session_id: "parent".to_string() },
            PromptOrigin::SchedulerFired,
            PromptOrigin::PlanResume,
        ] {
            let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
            seed_active_workflow_run(&actor, "selected").await;
            declare_workflow_wait(&actor, &["selected"]).await;
            let (respond_to, _response) = tokio::sync::oneshot::channel();
            assert!(!actor.queue_input(QueueInputRequest::from_input_origin(
                Vec::new(), "new-input".to_string(), InputOrigin::new(origin), PromptMode::Agent, respond_to,
            )).await);
            assert!(!actor.declared_wait_blocks_continuation().await);
            assert!(!actor.snapshot_turn_wait().await.is_empty());
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn wait_tool_rejects_stale_unknown_empty_and_nonowned_targets() {
    tokio::task::LocalSet::new().run_until(async {
        let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
        let (other, _other_gateway, _other_persistence) = actor_with_channels().await;
        seed_active_workflow_run(&other, "other-session-run").await;
        seed_active_workflow_run(&actor, "stale").await;
        actor.workflow_tracker().await.lock().interrupt("stale", "finished");
        for arguments in [
            r#"{"targets":[{"kind":"workflow_run","run_id":"other-session-run"}]}"#,
            r#"{"targets":[{"kind":"workflow_run","run_id":"stale"}]}"#,
            r#"{"targets":[{"kind":"bash_task","task_id":"unknown"}]}"#,
            r#"{"targets":[{"kind":"monitor","task_id":"unknown"}]}"#,
            r#"{"targets":[{"kind":"subagent","subagent_id":"unknown"}]}"#,
            r#"{"targets":[]}"#,
            r#"{"targets":[{"kind":"invented","id":"unknown"}]}"#,
        ] {
            let mut calls = vec![xai_grok_sampling_types::conversation::ToolCall {
                id: "invalid-wait".into(), name: DECLARE_WAIT_TOOL.to_string(), arguments: arguments.into(),
            }];
            actor.chat_state_handle.push_assistant_response(ConversationItem::assistant_tool_calls(calls.clone()));
            assert!(matches!(actor.handle_wait_tool_call(&mut calls).await, WaitToolStep::Retry), "accepted invalid declaration: {arguments}");
            assert!(!actor.declared_wait_blocks_continuation().await);
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn input_during_target_validation_cannot_rearm_a_wait() {
    tokio::task::LocalSet::new().run_until(async {
        let (mut actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
        seed_active_workflow_run(&actor, "selected").await;
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SubagentEvent>();
        actor.tool_context.subagent_event_tx = Some(event_tx);
        let (snapshot_started, started) = tokio::sync::oneshot::channel();
        let (release_snapshot, released) = tokio::sync::oneshot::channel();
        tokio::task::spawn_local(async move {
            if let Some(SubagentEvent::ListActive(request)) = event_rx.recv().await {
                snapshot_started.send(()).unwrap();
                released.await.unwrap();
                request.respond_to.send(Vec::new()).unwrap();
            }
        });
        let mut calls = vec![xai_grok_sampling_types::conversation::ToolCall {
            id: "declare-wait".into(), name: DECLARE_WAIT_TOOL.to_string(),
            arguments: r#"{"targets":[{"kind":"workflow_run","run_id":"selected"}]}"#.into(),
        }];
        actor.chat_state_handle.push_assistant_response(ConversationItem::assistant_tool_calls(calls.clone()));
        let declaring = actor.handle_wait_tool_call(&mut calls);
        let input = async {
            started.await.unwrap();
            let (respond_to, _response) = tokio::sync::oneshot::channel();
            assert!(!actor.queue_input(QueueInputRequest::from_input_origin(
                Vec::new(), "parent-input".to_string(),
                InputOrigin::new(PromptOrigin::ParentHumanMessage { message_id: "message".to_string(), sender_session_id: "parent".to_string() }),
                PromptMode::Agent, respond_to,
            )).await);
            release_snapshot.send(()).unwrap();
        };
        let (step, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(declaring, input)
        }).await.expect("wait validation must settle after the concurrent input");
        assert!(matches!(step, WaitToolStep::Retry));
        assert!(actor.turn_wait_declaration.borrow().declaration.is_empty());
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn subagent_sessions_never_declare_a_wait() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _gateway_rx, _persistence_rx) = actor_with_channels().await;
            seed_active_workflow_run(&actor, "wf-child-1").await;
            actor.startup_hints.is_subagent = true;
            let wait = actor.snapshot_turn_wait().await;
            assert!(
                wait.is_empty(),
                "a subagent's parent drives its lifecycle; it must not declare a wait, got {wait:?}"
            );
        })
        .await;
}
