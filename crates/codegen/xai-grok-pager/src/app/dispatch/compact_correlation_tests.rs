use super::*;
use crate::acp::tracker::TurnActivity;
use crate::app::agent::CompactInvocationSettlement;
use xai_grok_shell::extensions::notification::AutoCompactCancelReason;

#[derive(Clone, Copy)]
enum RpcArrival {
    BeforeLifecycle,
    BetweenTerminals,
    AfterLifecycle,
}

fn terminals() -> [SessionUpdate; 3] {
    [
        SessionUpdate::AutoCompactCompleted {
            tokens_before: Some(90_000), tokens_after: 40_000, elapsed_ms: Some(5), summary_preview: None,
        },
        SessionUpdate::AutoCompactCancelled { reason: AutoCompactCancelReason::UserCancelled },
        SessionUpdate::AutoCompactFailed { error: "generation failed".into() },
    ]
}

fn start_inline(app: &mut AppView) {
    notify(app, SessionUpdate::AutoCompactStarted {
        phase: CompactPhase::Executing, tokens_used: 90_000, context_window: 131_072,
        percentage: 68, reason: "automatic threshold".into(),
    });
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().session.turn_activity(), Some(TurnActivity::AutoCompacting));
}

#[test]
fn command_lifecycle_duplicates_wait_for_the_original_rpc_in_both_orders() {
    for lifecycle_first in [false, true] {
        for (outcome, terminal) in [outcomes()[0].clone(), outcomes()[1].clone(), outcomes()[3].clone()]
            .into_iter().zip(terminals()) {
            let mut app = test_app_with_agent();
            let id = AgentId(0);
            let invocation_id = submit(&mut app);
            if lifecycle_first {
                notify_manual(&mut app, invocation_id, terminal.clone());
                notify_manual(&mut app, invocation_id, terminal.clone());
                assert!(app.agents.get(&id).unwrap().session.state.is_compact_running());
                assert_eq!(app.agents.get(&id).unwrap().compact_invocations[&invocation_id].settlement,
                    CompactInvocationSettlement::LifecycleObserved);
                assert_eq!(events(&app).len(), 1, "lifecycle cannot claim the command's original responder");
            }
            dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                agent_id: id, invocation_id, result: outcome.clone(),
            }), &mut app);
            assert!(app.agents.get(&id).unwrap().session.state.is_idle());
            let settled = events(&app).len();
            assert_eq!(settled, 2, "one local start and exactly one terminal");
            for _ in 0..2 {
                notify_manual(&mut app, invocation_id, terminal.clone());
                dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                    agent_id: id, invocation_id, result: outcome.clone(),
                }), &mut app);
            }
            assert_eq!(events(&app).len(), settled);
            assert_eq!(app.agents.get(&id).unwrap().compact_invocations[&invocation_id].settlement,
                CompactInvocationSettlement::RpcSettled);
        }
    }
}

#[test]
fn old_correlated_lifecycle_cannot_finish_a_later_command_across_binding_epochs() {
    for transition in [CompactBindingTransition::Same, CompactBindingTransition::AwayAndBack,
        CompactBindingTransition::UnbindAndRebind] {
        for terminal in terminals() {
            let mut app = test_app_with_agent();
            let id = AgentId(0);
            let original = app.agents.get(&id).unwrap().session.session_id.clone().unwrap();
            let older = submit(&mut app);
            dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                agent_id: id, invocation_id: older, result: Ok(()),
            }), &mut app);
            let agent = app.agents.get_mut(&id).unwrap();
            match transition {
                CompactBindingTransition::Same => agent.bind_session_id(original),
                CompactBindingTransition::AwayAndBack => {
                    agent.bind_session_id(acp::SessionId::new("away"));
                    agent.bind_session_id(original);
                }
                CompactBindingTransition::UnbindAndRebind => {
                    agent.unbind_session_id();
                    agent.bind_session_id(original);
                }
            }
            let later = submit(&mut app);
            let before = events(&app).len();
            for _ in 0..2 { notify_manual(&mut app, older, terminal.clone()); }
            notify_with_meta(&mut app, terminal, Some(serde_json::json!({
                "compactInvocationId": older, "isReplay": true,
            })));
            dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                agent_id: id, invocation_id: older, result: Ok(()),
            }), &mut app);
            let agent = app.agents.get(&id).unwrap();
            assert!(agent.session.state.is_compact_running());
            assert_eq!(agent.compact_invocations[&later].settlement, CompactInvocationSettlement::AwaitingRpc);
            assert_eq!(events(&app).len(), before);
            dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                agent_id: id, invocation_id: later, result: Ok(()),
            }), &mut app);
            assert!(app.agents.get(&id).unwrap().session.state.is_idle());
            assert_eq!(events(&app).len(), before + 1);
        }
    }
}

#[test]
fn correlated_rejection_precedes_new_inline_automatic_ownership() {
    for rpc_first in [false, true] {
        for terminal in terminals() {
            let mut app = test_app_with_agent();
            let id = AgentId(0);
            dispatch(Action::SendPrompt("foreground".into()), &mut app);
            let prompt_id = app.agents.get(&id).unwrap().session.current_prompt_id.clone().unwrap();
            let manual = submit(&mut app);
            if rpc_first {
                dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                    agent_id: id, invocation_id: manual, result: Ok(()),
                }), &mut app);
            }
            start_inline(&mut app);
            let agent = app.agents.get(&id).unwrap();
            assert!(agent.session.in_flight_prompt.is_none());
            assert!(agent.session.compact_held_prompt.is_some());
            let started = agent.turn_started_at;
            let activity_started = agent.activity_started_at;
            let before = events(&app).len();
            for phase in [CompactPhase::Queued, CompactPhase::Executing] {
                notify_manual(&mut app, manual, SessionUpdate::AutoCompactStarted {
                    phase, tokens_used: 1, context_window: 2, percentage: 50,
                    reason: "old tagged start".into(),
                });
            }
            for _ in 0..2 { notify_manual(&mut app, manual, terminal.clone()); }
            notify_with_meta(&mut app, terminal.clone(), Some(serde_json::json!({
                "compactInvocationId": CompactInvocationId(uuid::Uuid::new_v4()),
            })));
            notify_with_meta(&mut app, terminal, Some(serde_json::json!({ "compactInvocationId": "invalid" })));
            let agent = app.agents.get(&id).unwrap();
            assert_eq!(events(&app).len(), before);
            assert_eq!(agent.session.turn_activity(), Some(TurnActivity::AutoCompacting));
            assert_eq!(agent.turn_started_at, started);
            assert_eq!(agent.activity_started_at, activity_started);
            assert_eq!(agent.session.current_prompt_id.as_deref(), Some(prompt_id.as_str()));
            assert!(agent.session.compact_held_prompt.is_some());
            notify(&mut app, terminals()[0].clone());
            let agent = app.agents.get(&id).unwrap();
            assert_ne!(agent.session.turn_activity(), Some(TurnActivity::AutoCompacting));
            assert!(agent.session.compact_held_prompt.is_none());
            assert!(agent.session.state.is_turn_running());
            if !rpc_first {
                dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                    agent_id: id, invocation_id: manual, result: Ok(()),
                }), &mut app);
            }
            complete_foreground(&mut app, prompt_id, acp::PromptResponse::new(acp::StopReason::EndTurn));
            let rendered = events(&app);
            assert_eq!(rendered.iter().filter(|event| matches!(event, SessionEvent::CompactCompleted { .. })).count(), 1);
            assert_eq!(rendered.iter().filter(|event| matches!(event, SessionEvent::CompactionCompleted { .. })).count(), 1,
                "the untagged automatic completion remains independently owned and drains on its real foreground");
        }
    }
}

#[test]
fn completed_then_failed_lifecycle_keeps_rpc_authoritative_exactly_once_in_both_orders() {
    for arrival in [RpcArrival::BeforeLifecycle, RpcArrival::BetweenTerminals, RpcArrival::AfterLifecycle] {
        for outcome in [outcomes()[0].clone(), outcomes()[3].clone()] {
            let mut app = test_app_with_agent();
            let id = AgentId(0);
            let invocation_id = submit(&mut app);
            let lifecycle = [terminals()[0].clone(), SessionUpdate::AutoCompactFailed {
                error: "own persistence acknowledgement lost".into(),
            }];
            if matches!(arrival, RpcArrival::BeforeLifecycle) {
                dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                    agent_id: id, invocation_id, result: outcome.clone(),
                }), &mut app);
            }
            notify_manual(&mut app, invocation_id, lifecycle[0].clone());
            if matches!(arrival, RpcArrival::BetweenTerminals) {
                assert_eq!(events(&app).len(), 1);
                assert!(app.agents.get(&id).unwrap().session.state.is_compact_running());
                dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                    agent_id: id, invocation_id, result: outcome.clone(),
                }), &mut app);
            }
            notify_manual(&mut app, invocation_id, lifecycle[1].clone());
            if matches!(arrival, RpcArrival::AfterLifecycle) {
                assert_eq!(events(&app).len(), 1);
                assert!(app.agents.get(&id).unwrap().session.state.is_compact_running());
                dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                    agent_id: id, invocation_id, result: outcome.clone(),
                }), &mut app);
            }
            for terminal in lifecycle {
                notify_manual(&mut app, invocation_id, terminal);
            }
            dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                agent_id: id, invocation_id, result: outcome.clone(),
            }), &mut app);
            let rendered = events(&app);
            assert_eq!(rendered.len(), 2, "one local start and one RPC-owned outcome");
            assert_eq!(rendered.iter().filter(|event| matches!(event, SessionEvent::CompactStarted)).count(), 1);
            assert_eq!(rendered.iter().filter(|event| matches!(event, SessionEvent::CompactionCompleted { .. })).count(), 0);
            match outcome {
                Ok(()) => {
                    assert_eq!(rendered.iter().filter(|event| matches!(event, SessionEvent::CompactCompleted { .. })).count(), 1);
                    assert_eq!(rendered.iter().filter(|event| matches!(event, SessionEvent::CompactionFailed { .. })).count(), 0);
                }
                Err(_) => {
                    assert_eq!(rendered.iter().filter(|event| matches!(event, SessionEvent::CompactCompleted { .. })).count(), 0);
                    assert_eq!(rendered.iter().filter(|event| matches!(event, SessionEvent::CompactionFailed { .. })).count(), 1);
                }
            }
            let agent = app.agents.get(&id).unwrap();
            assert!(agent.session.state.is_idle());
            assert_eq!(agent.compact_invocations[&invocation_id].settlement, CompactInvocationSettlement::RpcSettled);
        }
    }
}

#[test]
fn admission_error_without_lifecycle_settles_without_consuming_later_automatic_events() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    let rejected = submit(&mut app);
    dispatch(Action::TaskComplete(TaskResult::CompactComplete {
        agent_id: id, invocation_id: rejected, result: outcomes()[2].clone(),
    }), &mut app);
    assert!(app.agents.get(&id).unwrap().session.state.is_idle());
    assert_eq!(events(&app).iter().filter(|event| matches!(event, SessionEvent::CompactionFailed { .. })).count(), 1);
    dispatch(Action::SendPrompt("later foreground".into()), &mut app);
    start_inline(&mut app);
    notify(&mut app, terminals()[2].clone());
    assert!(app.agents.get(&id).unwrap().session.state.is_turn_running());
    assert_ne!(app.agents.get(&id).unwrap().session.turn_activity(), Some(TurnActivity::AutoCompacting));
    assert_eq!(events(&app).iter().filter(|event| matches!(event, SessionEvent::CompactionFailed { .. })).count(), 2);
}
