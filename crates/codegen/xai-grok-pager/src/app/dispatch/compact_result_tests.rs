use super::*;

#[path = "compact_correlation_tests.rs"]
mod correlation_tests;
use crate::app::actions::{Action, TaskResult};
use crate::app::agent::CompactInvocationId;
use crate::app::dispatch::router::dispatch;
use crate::app::dispatch::tests::test_app_with_agent;
use xai_grok_shell::extensions::notification::{CompactPhase, SessionNotification, SessionUpdate};
use xai_grok_shell::session::helpers::session_compact::{CompactErrorKind, compact_error_data};

fn submit(app: &mut AppView) -> CompactInvocationId {
    let effects = dispatch(Action::SendPrompt("/compact".into()), app);
    match effects.as_slice() {
        [Effect::Compact { invocation_id, .. }] => *invocation_id,
        other => panic!("expected original compact RPC, got {other:?}"),
    }
}

fn complete_foreground(app: &mut AppView, prompt_id: String, response: acp::PromptResponse) {
    dispatch(Action::TaskComplete(TaskResult::PromptResponse {
        agent_id: AgentId(0), result: Ok(response), http_status: None, prompt_id: Some(prompt_id),
    }), app);
}

fn events(app: &AppView) -> Vec<SessionEvent> {
    let agent = app.agents.get(&AgentId(0)).unwrap();
    (0..agent.scrollback.len()).filter_map(|i| match &agent.scrollback.entry(i)?.block {
        RenderBlock::SessionEvent(event) => Some(event.event.clone()),
        _ => None,
    }).collect()
}

fn outcomes() -> [Result<(), crate::app::effects::CompactError>; 4] {
    [
        Ok(()),
        Err(crate::app::effects::compact_error(&acp::Error::internal_error().data(
            compact_error_data(CompactErrorKind::Cancelled, "cancelled")))),
        Err(crate::app::effects::compact_error(&acp::Error::invalid_request().data(
            serde_json::json!({"kind": "compact_busy", "message": "admission rejected"})))),
        Err(crate::app::effects::compact_error(&acp::Error::internal_error().data("transport closed"))),
    ]
}

#[test]
fn busy_compact_original_results_settle_once_before_or_after_foreground_ends() {
    for foreground_ends_first in [false, true] {
        for outcome in &outcomes() {
            let mut app = test_app_with_agent();
            dispatch(Action::SendPrompt("original foreground".into()), &mut app);
            let id = AgentId(0);
            let prompt_id = app.agents.get(&id).unwrap().session.current_prompt_id.clone().unwrap();
            let invocation_id = submit(&mut app);
            if foreground_ends_first {
                complete_foreground(&mut app, prompt_id.clone(), acp::PromptResponse::new(acp::StopReason::EndTurn));
            }
            let started = app.agents.get(&id).unwrap().turn_started_at;
            let activity = app.agents.get(&id).unwrap().activity_started_at;
            dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                agent_id: id, invocation_id, result: outcome.clone(),
            }), &mut app);
            let agent = &app.agents.get(&id).unwrap();
            assert_eq!(agent.session.state.is_turn_running(), !foreground_ends_first);
            assert_eq!(agent.turn_started_at, started);
            assert_eq!(agent.activity_started_at, activity);
            if !foreground_ends_first {
                assert_eq!(agent.session.current_prompt_id.as_deref(), Some(prompt_id.as_str()));
                assert!(agent.session.in_flight_prompt.is_some());
            }
            assert!(agent.compact_invocations.values().all(|invocation|
                invocation.settlement == crate::app::agent::CompactInvocationSettlement::RpcSettled));
            let settled = events(&app);
            assert_eq!(settled.iter().filter(|e| matches!(e, SessionEvent::CompactStarted)).count(), 1);
            match outcome {
                Ok(()) => assert_eq!(settled.iter().filter(|e| matches!(e, SessionEvent::CompactCompleted { .. })).count(), 1),
                Err(error) if error.cancelled => assert_eq!(settled.iter().filter(|e| matches!(e, SessionEvent::CompactionCancelled)).count(), 1),
                Err(_) => assert_eq!(settled.iter().filter(|e| matches!(e, SessionEvent::CompactionFailed { .. })).count(), 1),
            }
            dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                agent_id: id, invocation_id, result: outcome.clone(),
            }), &mut app);
            assert_eq!(events(&app).len(), settled.len(), "duplicate RPC result must not settle twice");
        }
    }
}

#[test]
fn older_busy_compact_reply_cannot_finish_a_later_compact_command() {
    for outcome in outcomes() {
        let mut app = test_app_with_agent();
        let id = AgentId(0);
        dispatch(Action::SendPrompt("foreground".into()), &mut app);
        let prompt_id = app.agents.get(&id).unwrap().session.current_prompt_id.clone().unwrap();
        let older = submit(&mut app);
        complete_foreground(&mut app, prompt_id, acp::PromptResponse::new(acp::StopReason::EndTurn));
        let later = submit(&mut app);
        dispatch(Action::TaskComplete(TaskResult::CompactComplete {
            agent_id: id, invocation_id: older, result: outcome,
        }), &mut app);
        assert!(app.agents.get(&id).unwrap().session.state.is_compact_running());
        assert!(app.agents.get(&id).unwrap().compact_invocations.contains_key(&later));
        dispatch(Action::TaskComplete(TaskResult::CompactComplete {
            agent_id: id, invocation_id: later, result: Ok(()),
        }), &mut app);
        assert!(app.agents.get(&id).unwrap().session.state.is_idle());
        assert_eq!(events(&app).iter().filter(|e| matches!(e,
            SessionEvent::CompactCompleted { .. } | SessionEvent::CompactionCancelled | SessionEvent::CompactionFailed { .. }
        )).count(), 2);
    }
}

fn notify(app: &mut AppView, update: SessionUpdate) {
    notify_with_meta(app, update, None);
}

fn notify_manual(app: &mut AppView, invocation_id: CompactInvocationId, update: SessionUpdate) {
    notify_with_meta(app, update, Some(serde_json::json!({ "compactInvocationId": invocation_id })));
}

fn notify_with_meta(app: &mut AppView, update: SessionUpdate, meta: Option<serde_json::Value>) {
    let payload = SessionNotification {
        session_id: app.agents.get(&AgentId(0)).unwrap().session.session_id.clone().unwrap(),
        update,
        meta,
    };
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    crate::app::acp_handler::handle(xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
        request: xai_grok_test_support::acp_fixtures::ext_notification("x.ai/session_notification", &payload),
        response_tx,
    }), app);
}

#[derive(Clone, Copy)]
enum CompactBindingTransition {
    Same,
    AwayAndBack,
    UnbindAndRebind,
}

#[test]
fn compact_reply_obeys_actual_session_bind_unbind_rebind_boundaries() {
    for transition in [CompactBindingTransition::Same, CompactBindingTransition::AwayAndBack,
        CompactBindingTransition::UnbindAndRebind] {
        for outcome in outcomes() {
            let mut app = test_app_with_agent();
            let id = AgentId(0);
            let original = app.agents.get(&id).unwrap().session.session_id.clone().unwrap();
            dispatch(Action::SendPrompt("foreground across binding changes".into()), &mut app);
            let prompt_id = app.agents.get(&id).unwrap().session.current_prompt_id.clone().unwrap();
            let older = submit(&mut app);
            let agent = app.agents.get_mut(&id).unwrap();
            match transition {
                CompactBindingTransition::Same => agent.bind_session_id(original),
                CompactBindingTransition::AwayAndBack => {
                    agent.bind_session_id(acp::SessionId::new("different-compact-session"));
                    agent.bind_session_id(original);
                }
                CompactBindingTransition::UnbindAndRebind => {
                    agent.unbind_session_id();
                    agent.bind_session_id(original);
                }
            }
            let before = events(&app).len();
            dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                agent_id: id, invocation_id: older, result: outcome,
            }), &mut app);
            match transition {
                CompactBindingTransition::Same => {
                    assert!(app.agents.get(&id).unwrap().session.state.is_turn_running());
                    assert_eq!(events(&app).len(), before + 1);
                }
                CompactBindingTransition::AwayAndBack | CompactBindingTransition::UnbindAndRebind => {
                    assert_eq!(events(&app).len(), before, "a returned session id cannot revive an old RPC owner");
                    assert!(app.agents.get(&id).unwrap().session.state.is_turn_running());
                    let current = submit(&mut app);
                    dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                        agent_id: id, invocation_id: older, result: Ok(()),
                    }), &mut app);
                    assert!(app.agents.get(&id).unwrap().compact_invocations.contains_key(&current));
                    assert!(app.agents.get(&id).unwrap().session.state.is_turn_running());
                    dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                        agent_id: id, invocation_id: current, result: Ok(()),
                    }), &mut app);
                    assert!(app.agents.get(&id).unwrap().session.state.is_turn_running());
                }
            }
            assert!(app.agents.get(&id).unwrap().compact_invocations.values().all(|invocation|
                invocation.settlement == crate::app::agent::CompactInvocationSettlement::RpcSettled));
            assert_eq!(app.agents.get(&id).unwrap().session.current_prompt_id.as_deref(), Some(prompt_id.as_str()));
            assert!(app.agents.get(&id).unwrap().session.in_flight_prompt.is_some());
            complete_foreground(&mut app, prompt_id, acp::PromptResponse::new(acp::StopReason::EndTurn));
            assert!(app.agents.get(&id).unwrap().session.state.is_idle());
        }
    }
}

#[test]
fn manual_compact_lifecycle_before_or_after_rpc_has_one_terminal_outcome() {
    use xai_grok_shell::extensions::notification::AutoCompactCancelReason;

    let terminals = [
        (Ok(()), SessionUpdate::AutoCompactCompleted {
            tokens_before: Some(90_000), tokens_after: 40_000, elapsed_ms: None, summary_preview: None,
        }),
        (Err(crate::app::effects::compact_error(&acp::Error::internal_error().data(
            compact_error_data(CompactErrorKind::Cancelled, "cancelled")))),
            SessionUpdate::AutoCompactCancelled { reason: AutoCompactCancelReason::UserCancelled }),
        (Err(crate::app::effects::compact_error(&acp::Error::internal_error().data(
            compact_error_data(CompactErrorKind::Failed, "generation failed")))),
            SessionUpdate::AutoCompactFailed { error: "generation failed".into() }),
    ];
    for lifecycle_first in [false, true] {
        for foreground_ends_first in [false, true] {
            for (outcome, terminal) in &terminals {
                let mut app = test_app_with_agent();
                let id = AgentId(0);
                dispatch(Action::SendPrompt("original foreground".into()), &mut app);
                let prompt_id = app.agents.get(&id).unwrap().session.current_prompt_id.clone().unwrap();
                let invocation_id = submit(&mut app);
                notify_manual(&mut app, invocation_id, SessionUpdate::AutoCompactStarted {
                    phase: CompactPhase::Queued, tokens_used: 90_000, context_window: 131_072,
                    percentage: 68, reason: "waiting for preceding boundary".into(),
                });
                if foreground_ends_first {
                    complete_foreground(&mut app, prompt_id.clone(), acp::PromptResponse::new(acp::StopReason::EndTurn));
                }
                let started = app.agents.get(&id).unwrap().turn_started_at;
                let activity = app.agents.get(&id).unwrap().session.turn_activity();
                if lifecycle_first {
                    notify_manual(&mut app, invocation_id, terminal.clone());
                    notify_manual(&mut app, invocation_id, terminal.clone());
                }
                dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                    agent_id: id, invocation_id, result: outcome.clone(),
                }), &mut app);
                if !lifecycle_first { notify_manual(&mut app, invocation_id, terminal.clone()); }
                notify_manual(&mut app, invocation_id, terminal.clone());
                let agent = app.agents.get(&id).unwrap();
                assert_eq!(agent.session.state.is_turn_running(), !foreground_ends_first);
                assert_eq!(agent.turn_started_at, started);
                assert_eq!(agent.session.turn_activity(), activity);
                if !foreground_ends_first {
                    assert_eq!(agent.session.current_prompt_id.as_deref(), Some(prompt_id.as_str()));
                    assert!(agent.session.in_flight_prompt.is_some());
                    complete_foreground(&mut app, prompt_id, acp::PromptResponse::new(acp::StopReason::EndTurn));
                }
                let settled = events(&app);
                assert_eq!(settled.iter().filter(|event| matches!(event,
                    SessionEvent::CompactCompleted { .. } | SessionEvent::CompactionCompleted { .. }
                        | SessionEvent::CompactionCancelled | SessionEvent::CompactionFailed { .. }
                )).count(), 1, "lifecycle and original RPC must share one terminal outcome");
                assert_eq!(settled.iter().filter(|event| matches!(event, SessionEvent::CompactStarted)).count(), 1);
                assert!(!settled.iter().any(|event| matches!(event, SessionEvent::CompactionStarted { .. })));
                dispatch(Action::TaskComplete(TaskResult::CompactComplete {
                    agent_id: id, invocation_id, result: outcome.clone(),
                }), &mut app);
                assert_eq!(events(&app).len(), settled.len());
            }
        }
    }
}

#[test]
fn compact_phase_extension_defaults_legacy_notifications_to_executing() {
    let legacy = serde_json::json!({
        "sessionUpdate": "auto_compact_started", "tokens_used": 90_000,
        "context_window": 131_072, "percentage": 68, "reason": "legacy",
    });
    let update: SessionUpdate = serde_json::from_value(legacy).unwrap();
    assert!(matches!(update, SessionUpdate::AutoCompactStarted { phase: CompactPhase::Executing, .. }));
}

#[test]
fn queued_compact_notification_preserves_prompt_images_chips_for_hook_denial() {
    let mut app = test_app_with_agent();
    let id = AgentId(0);
    app.agents.get_mut(&id).unwrap().prompt.insert_image(crate::prompt_images::PastedImage {
        element_id: xai_ratatui_textarea::ElementId::from_raw(0), display_number: 0,
        mime_type: "image/png".into(), dimensions: Some((8, 8)), byte_len: 1,
        encoded_bytes: Some(vec![0].into()), source_path: None, staged_temp_path: None,
        session_image_path: None, preview: crate::prompt_images::PromptImagePreview::default(),
    });
    let text = format!("{} inspect attached image", app.agents.get(&id).unwrap().prompt.text());
    dispatch(Action::SendPrompt(text), &mut app);
    let agent = &app.agents.get(&id).unwrap();
    let prompt_id = agent.session.current_prompt_id.clone().unwrap();
    let stash = agent.session.in_flight_prompt.as_ref().unwrap().clone();
    assert_eq!(stash.images.len(), 1);
    assert!(!stash.chip_elements.is_empty());
    let invocation_id = submit(&mut app);
    let started = app.agents.get(&id).unwrap().turn_started_at;
    let activity = app.agents.get(&id).unwrap().session.turn_activity();
    let payload = SessionNotification {
        session_id: app.agents.get(&id).unwrap().session.session_id.clone().unwrap(),
        update: SessionUpdate::AutoCompactStarted {
            phase: CompactPhase::Queued, tokens_used: 90_000, context_window: 131_072,
            percentage: 68, reason: "not an executing compact".into(),
        }, meta: None,
    };
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    crate::app::acp_handler::handle(xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
        request: xai_grok_test_support::acp_fixtures::ext_notification("x.ai/session_notification", &payload),
        response_tx,
    }), &mut app);
    let agent = &app.agents.get(&id).unwrap();
    assert_eq!(agent.turn_started_at, started);
    assert_eq!(agent.session.turn_activity(), activity);
    assert!(agent.session.compact_held_prompt.is_none());
    assert_eq!(agent.session.in_flight_prompt.as_ref().unwrap().images.len(), 1);
    complete_foreground(&mut app, prompt_id, acp::PromptResponse::new(acp::StopReason::Cancelled).meta(
        serde_json::json!({"cancellationCategory": xai_grok_shell::session::commands::HOOK_DENIED_CATEGORY}).as_object().cloned(),
    ));
    let agent = &app.agents.get(&id).unwrap();
    assert!(agent.session.hook_block_hold);
    assert!(agent.session.blocked_prompt.is_some());
    assert!(agent.question_view.is_some());
    let restored = agent.session.pending_prompts.front().unwrap();
    assert_eq!(restored.text, stash.text);
    assert_eq!(restored.images.len(), stash.images.len());
    assert_eq!(restored.images[0].mime_type, stash.images[0].mime_type);
    assert_eq!(restored.images[0].encoded_bytes, stash.images[0].encoded_bytes);
    assert_eq!(restored.chip_elements.len(), stash.chip_elements.len());
    assert_eq!(restored.chip_elements[0].range, stash.chip_elements[0].range);
    dispatch(Action::TaskComplete(TaskResult::CompactComplete {
        agent_id: id, invocation_id, result: Ok(()),
    }), &mut app);
    assert_eq!(events(&app).iter().filter(|e| matches!(e, SessionEvent::CompactStarted)).count(), 1);
    assert_eq!(events(&app).iter().filter(|e| matches!(e, SessionEvent::CompactionStarted { .. })).count(), 0);
}
