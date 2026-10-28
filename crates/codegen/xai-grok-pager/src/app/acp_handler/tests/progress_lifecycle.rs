use super::*;
use crate::acp::tracker::{TurnActivity, WaitingReason};
use crate::app::actions::{Action, Effect, TaskResult};
use crate::app::agent::AgentCommand;
use xai_grok_shell::extensions::notification::{CompactPhase, RetryState, SessionNotification};
use xai_grok_shell::session::acp_types::{AutomaticCompactInvocation, CompactInvocationId};

fn deliver(app: &mut AppView, session: &str, prompt: &str, update: acp::SessionUpdate) {
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    handle(AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs {
        request: acp::SessionNotification::new(acp::SessionId::new(session), update)
            .meta(Some(serde_json::json!({ "promptId": prompt }).as_object().unwrap().clone())),
        response_tx,
    }), app);
}

fn lifecycle(app: &mut AppView, session: &str, update: XaiSessionUpdate, meta: serde_json::Value) {
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    handle(AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
        request: acp_fixtures::ext_notification("x.ai/session_notification", &SessionNotification {
            session_id: acp::SessionId::new(session), update, meta: Some(meta),
        }),
        response_tx,
    }), app);
}

fn screen(app: &mut AppView, id: AgentId) -> String {
    let area = ratatui::layout::Rect::new(0, 0, 120, 35);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    app.agents.get_mut(&id).unwrap().draw(
        area, &mut buffer, &crate::actions::ActionRegistry::defaults(),
        &mut crate::scrollback::render::ScratchBuffer::new(), None, false,
        crate::app::agent_view::BannerSlotParams::none(), false, &mut Vec::new(),
        crate::app::agent_view::AppRenderParams::default(),
    );
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

fn started(phase: CompactPhase) -> XaiSessionUpdate {
    XaiSessionUpdate::AutoCompactStarted {
        phase, tokens_used: 90_000, context_window: 100_000, percentage: 90, reason: "threshold".into(),
    }
}

fn completed() -> XaiSessionUpdate {
    XaiSessionUpdate::AutoCompactCompleted {
        tokens_before: Some(90_000), tokens_after: 20_000, elapsed_ms: Some(100), summary_preview: None,
    }
}

fn automatic(invocation: &AutomaticCompactInvocation, session: &str, sequence: u64) -> serde_json::Value {
    serde_json::json!({ "automaticCompactInvocation": invocation, "eventId": format!("{session}-{sequence}") })
}

fn edit() -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCall(acp::ToolCall::new(acp::ToolCallId::new("edit-progress"), "Edit file")
        .kind(acp::ToolKind::Edit).status(acp::ToolCallStatus::InProgress))
}

fn edit_status(status: acp::ToolCallStatus) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
        acp::ToolCallId::new("edit-progress"), acp::ToolCallUpdateFields::new().status(Some(status)),
    ))
}

fn tool_start(id: &str, title: &str) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCall(
        acp::ToolCall::new(acp::ToolCallId::new(id), title)
            .kind(acp::ToolKind::Edit)
            .status(acp::ToolCallStatus::InProgress),
    )
}

fn tool_update(id: &str, fields: acp::ToolCallUpdateFields) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
        acp::ToolCallId::new(id),
        fields,
    ))
}

#[test]
fn hidden_reasoning_and_early_edit_progress_leave_the_first_response_wait() {
    let mut app = make_app_with_agent("progress");
    crate::app::dispatch::dispatch(Action::SendPrompt("repair file".into()), &mut app);
    let prompt = test_agent(&app, AgentId(0)).session.current_prompt_id.clone().unwrap();
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::Waiting(WaitingReason::Model)));
    assert!(screen(&mut app, AgentId(0)).contains("Waiting for response…"));
    crate::appearance::cache::set_show_thinking_blocks(false);
    deliver(&mut app, "progress", &prompt, acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
        acp::ContentBlock::Text(acp::TextContent::new("Inspecting the file")),
    )));
    crate::appearance::cache::set_show_thinking_blocks(true);
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::Thinking));
    assert!(screen(&mut app, AgentId(0)).contains("Thinking…"));
    assert!(test_agent(&app, AgentId(0)).session.in_flight_prompt.is_none());

    deliver(&mut app, "progress", &prompt, edit_status(acp::ToolCallStatus::InProgress));
    deliver(&mut app, "progress", &prompt, edit());
    assert!(matches!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::ToolRunning { .. })));
    deliver(&mut app, "progress", &prompt, edit_status(acp::ToolCallStatus::Completed));
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::Waiting(WaitingReason::Model)));
    deliver(&mut app, "progress", &prompt, acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
        acp::ContentBlock::Text(acp::TextContent::new("File repaired")),
    )));
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::Responding));
    assert!(screen(&mut app, AgentId(0)).contains("Responding…"));
    crate::app::dispatch::dispatch(Action::TaskComplete(TaskResult::PromptResponse {
        agent_id: AgentId(0), prompt_id: Some(prompt), http_status: None,
        result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
    }), &mut app);
    assert!(test_agent(&app, AgentId(0)).session.state.is_idle());
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), None);
}

#[test]
fn automatic_compaction_outcomes_do_not_hide_edits_or_cross_session_owners() {
    for terminal in [completed(), XaiSessionUpdate::AutoCompactFailed { error: "fixture failure".into() },
        XaiSessionUpdate::AutoCompactCancelled {
            reason: xai_grok_shell::extensions::notification::AutoCompactCancelReason::UserCancelled,
        }] {
        let mut app = make_app_with_parent_and_child("progress-root", "progress-child");
        crate::app::dispatch::dispatch(Action::SendPrompt("repair file".into()), &mut app);
        let prompt = test_agent(&app, AgentId(0)).session.current_prompt_id.clone().unwrap();
        let invocation = AutomaticCompactInvocation {
            id: CompactInvocationId(uuid::Uuid::from_u128(71)), prompt_id: Some(prompt.clone()),
        };
        lifecycle(&mut app, "progress-root", started(CompactPhase::Executing), automatic(&invocation, "progress-root", 10));
        deliver(&mut app, "progress-root", &prompt, edit());
        assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::AutoCompacting));
        app.active_view = ActiveView::Agent(AgentId(1));
        lifecycle(&mut app, "progress-child", completed(), automatic(&invocation, "progress-child", 11));
        assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::AutoCompacting));
        lifecycle(&mut app, "progress-root", terminal, automatic(&invocation, "progress-root", 12));
        assert!(matches!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::ToolRunning { .. })));
        assert!(test_agent(&app, AgentId(0)).session.compact_held_prompt.is_none());
        assert!(test_agent(&app, AgentId(1)).session.state.is_idle());
        assert_eq!(app.active_view, ActiveView::Agent(AgentId(1)));
    }
}

#[test]
fn older_compaction_completion_replay_and_old_turn_progress_cannot_replace_live_activity() {
    let mut app = make_app_with_agent("owned-progress");
    crate::app::dispatch::dispatch(Action::SendPrompt("repair file".into()), &mut app);
    let prompt = test_agent(&app, AgentId(0)).session.current_prompt_id.clone().unwrap();
    let old = AutomaticCompactInvocation { id: CompactInvocationId(uuid::Uuid::from_u128(81)), prompt_id: Some(prompt.clone()) };
    let current = AutomaticCompactInvocation { id: CompactInvocationId(uuid::Uuid::from_u128(82)), prompt_id: Some(prompt.clone()) };
    lifecycle(&mut app, "owned-progress", started(CompactPhase::Executing), automatic(&old, "owned-progress", 10));
    lifecycle(&mut app, "owned-progress", completed(), automatic(&old, "owned-progress", 11));
    lifecycle(&mut app, "owned-progress", started(CompactPhase::Executing), automatic(&current, "owned-progress", 12));
    lifecycle(&mut app, "owned-progress", completed(), automatic(&old, "owned-progress", 13));
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::AutoCompacting));
    app.agents.get_mut(&AgentId(0)).unwrap().arm_late_replay_grace();
    let replay = serde_json::json!({ "automaticCompactInvocation": old, "eventId": "owned-progress-10", "isReplay": true });
    lifecycle(&mut app, "owned-progress", started(CompactPhase::Executing), replay);
    lifecycle(&mut app, "owned-progress", completed(), serde_json::json!({
        "automaticCompactInvocation": old, "eventId": "owned-progress-11", "isReplay": true,
    }));
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::AutoCompacting));
    lifecycle(&mut app, "owned-progress", completed(), automatic(&current, "owned-progress", 14));
    crate::app::dispatch::dispatch(Action::TaskComplete(TaskResult::PromptResponse {
        agent_id: AgentId(0), prompt_id: Some(prompt.clone()), http_status: None,
        result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
    }), &mut app);
    crate::app::dispatch::dispatch(Action::SendPrompt("next task".into()), &mut app);
    lifecycle(&mut app, "owned-progress", started(CompactPhase::Executing), automatic(&old, "owned-progress", 15));
    lifecycle(&mut app, "owned-progress", XaiSessionUpdate::ToolCallDeltaChunk {
        tool_call_id: None, tool_index: 0, name: Some("edit_file".into()), arguments_delta: Some("{}".into()),
    }, serde_json::json!({ "promptId": prompt }));
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::Waiting(WaitingReason::Model)));
}

#[test]
fn manual_compaction_displays_admission_execution_and_persistence_until_original_rpc_settles() {
    let mut app = make_app_with_agent("manual-progress");
    let effects = crate::app::dispatch::dispatch(Action::SendPrompt("/compact".into()), &mut app);
    let invocation = match effects.as_slice() {
        [Effect::Compact { invocation_id, .. }] => *invocation_id,
        other => panic!("expected compact effect, got {other:?}"),
    };
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::Waiting(WaitingReason::CompactionAdmission)));
    assert!(screen(&mut app, AgentId(0)).contains("Requesting compaction…"));
    for phase in [CompactPhase::Queued, CompactPhase::Executing] {
        lifecycle(&mut app, "manual-progress", started(phase), serde_json::json!({ "compactInvocationId": invocation }));
        assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(match phase {
            CompactPhase::Queued => TurnActivity::Waiting(WaitingReason::CompactionQueued),
            CompactPhase::Executing => TurnActivity::AutoCompacting,
        }));
        let expected = match phase {
            CompactPhase::Queued => "Compaction queued…",
            CompactPhase::Executing => "Compacting…",
        };
        assert!(screen(&mut app, AgentId(0)).contains(expected));
    }
    lifecycle(&mut app, "manual-progress", completed(), serde_json::json!({ "compactInvocationId": invocation }));
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::Waiting(WaitingReason::CompactionSettlement)));
    assert!(screen(&mut app, AgentId(0)).contains("Finishing compaction…"));
    assert!(matches!(test_agent(&app, AgentId(0)).session.state, AgentState::CommandRunning { command: AgentCommand::Compact, .. }));
    crate::app::dispatch::dispatch(Action::TaskComplete(TaskResult::CompactComplete {
        agent_id: AgentId(0), invocation_id: invocation, result: Ok(()),
    }), &mut app);
    assert!(test_agent(&app, AgentId(0)).session.state.is_idle());
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), None);
    assert!(!screen(&mut app, AgentId(0)).contains("Finishing compaction…"));
}

#[test]
fn queued_manual_request_takes_the_idle_loader_only_when_execution_really_starts() {
    let mut app = make_app_with_agent("queued-progress");
    crate::app::dispatch::dispatch(Action::SendPrompt("foreground task".into()), &mut app);
    let prompt = test_agent(&app, AgentId(0)).session.current_prompt_id.clone().unwrap();
    deliver(&mut app, "queued-progress", &prompt, edit());
    let effects = crate::app::dispatch::dispatch(Action::SendPrompt("/compact".into()), &mut app);
    let invocation = match effects.as_slice() {
        [Effect::Compact { invocation_id, .. }] => *invocation_id,
        other => panic!("expected compact effect, got {other:?}"),
    };
    lifecycle(&mut app, "queued-progress", started(CompactPhase::Queued), serde_json::json!({ "compactInvocationId": invocation }));
    assert!(matches!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::ToolRunning { .. })));
    crate::app::dispatch::dispatch(Action::TaskComplete(TaskResult::PromptResponse {
        agent_id: AgentId(0), prompt_id: Some(prompt), http_status: None,
        result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
    }), &mut app);
    lifecycle(&mut app, "queued-progress", started(CompactPhase::Executing), serde_json::json!({ "compactInvocationId": invocation }));
    assert!(matches!(test_agent(&app, AgentId(0)).session.state, AgentState::CommandRunning { command: AgentCommand::Compact, .. }));
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::AutoCompacting));
    lifecycle(&mut app, "queued-progress", started(CompactPhase::Queued), serde_json::json!({ "compactInvocationId": invocation }));
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::AutoCompacting));
    lifecycle(&mut app, "queued-progress", completed(), serde_json::json!({ "compactInvocationId": invocation }));
    crate::app::dispatch::dispatch(Action::TaskComplete(TaskResult::CompactComplete {
        agent_id: AgentId(0), invocation_id: invocation, result: Ok(()),
    }), &mut app);
    assert!(test_agent(&app, AgentId(0)).session.state.is_idle());
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), None);
}

#[test]
fn leading_tool_updates_survive_until_their_tool_call_and_keep_it_running() {
    let mut app = make_app_with_agent("orphan-progress");
    crate::app::dispatch::dispatch(Action::SendPrompt("repair file".into()), &mut app);
    let prompt = test_agent(&app, AgentId(0)).session.current_prompt_id.clone().unwrap();
    // Two partial leading updates arrive before the ToolCall: distinctive content, then status.
    deliver(&mut app, "orphan-progress", &prompt, tool_update("orphan-edit",
        acp::ToolCallUpdateFields::new().title(Some("Preparing orphan edit".into()))));
    deliver(&mut app, "orphan-progress", &prompt, tool_update("orphan-edit",
        acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::InProgress))));
    // The start carries neither the title nor a terminal status.
    deliver(&mut app, "orphan-progress", &prompt, tool_start("orphan-edit", "Edit file"));
    assert!(matches!(
        test_agent(&app, AgentId(0)).resolve_turn_activity(),
        Some(TurnActivity::ToolRunning { .. })
    ));
    assert!(screen(&mut app, AgentId(0)).contains("Preparing orphan edit"));
    // The orphan's nonterminal status must survive the merge: only a real terminal update completes it.
    deliver(&mut app, "orphan-progress", &prompt, tool_update("orphan-edit",
        acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed))));
    assert_eq!(
        test_agent(&app, AgentId(0)).resolve_turn_activity(),
        Some(TurnActivity::Waiting(WaitingReason::Model))
    );
    // The retained content survives into the completed row.
    assert!(screen(&mut app, AgentId(0)).contains("Preparing orphan edit"));

    // Terminal-before-start renders the finished call instead of reopening it as running.
    deliver(&mut app, "orphan-progress", &prompt, tool_update("orphan-done",
        acp::ToolCallUpdateFields::new()
            .title(Some("Already finished".into()))
            .status(Some(acp::ToolCallStatus::Failed))));
    deliver(&mut app, "orphan-progress", &prompt, tool_start("orphan-done", "Late start"));
    assert_eq!(
        test_agent(&app, AgentId(0)).resolve_turn_activity(),
        Some(TurnActivity::Waiting(WaitingReason::Model))
    );
    assert!(screen(&mut app, AgentId(0)).contains("Already finished"));
}

#[test]
fn live_auto_compaction_during_local_load_owns_the_live_state() {
    let mut app = make_app_with_agent("load-progress");
    crate::app::dispatch::dispatch(Action::SendPrompt("repair file".into()), &mut app);
    let prompt = test_agent(&app, AgentId(0)).session.current_prompt_id.clone().unwrap();
    let invocation = AutomaticCompactInvocation {
        id: CompactInvocationId(uuid::Uuid::from_u128(91)), prompt_id: Some(prompt.clone()),
    };
    // A wire-live start while the local load window is still open: history rendering must
    // not steal it from the live owner.
    app.agents.get_mut(&AgentId(0)).unwrap().session.loading_replay = true;
    lifecycle(&mut app, "load-progress", started(CompactPhase::Executing), automatic(&invocation, "load-progress", 10));
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::AutoCompacting));
    // Producer-stamped replay of an older invocation renders as history and settles nothing.
    let older = AutomaticCompactInvocation {
        id: CompactInvocationId(uuid::Uuid::from_u128(90)), prompt_id: Some(prompt.clone()),
    };
    lifecycle(&mut app, "load-progress", started(CompactPhase::Executing), serde_json::json!({
        "automaticCompactInvocation": older, "eventId": "load-progress-9", "isReplay": true,
    }));
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), Some(TurnActivity::AutoCompacting));
    app.agents.get_mut(&AgentId(0)).unwrap().session.loading_replay = false;
    // The owner's own terminal settles it after the window closes.
    lifecycle(&mut app, "load-progress", completed(), automatic(&invocation, "load-progress", 11));
    assert_eq!(
        test_agent(&app, AgentId(0)).resolve_turn_activity(),
        Some(TurnActivity::Waiting(WaitingReason::Model))
    );
}

#[test]
fn retry_countdown_rides_the_scheduler_deadline_and_replay_stays_history() {
    let mut app = make_app_with_agent("retry-progress");
    crate::app::dispatch::dispatch(Action::SendPrompt("repair file".into()), &mut app);
    let prompt = test_agent(&app, AgentId(0)).session.current_prompt_id.clone().unwrap();
    let retry = |attempt: u32, deadline_ms: i64| XaiSessionUpdate::RetryState(RetryState::Retrying {
        attempt, max_retries: 5, reason: "API error (status 503): overloaded".into(),
        error_type: Some("api".into()), next_attempt_at_ms: Some(deadline_ms),
    });
    let soon = chrono::Utc::now().timestamp_millis() + 12_500;
    lifecycle(&mut app, "retry-progress", retry(2, soon), serde_json::json!({}));
    assert!(matches!(
        test_agent(&app, AgentId(0)).resolve_turn_activity(),
        Some(TurnActivity::Retrying { attempt: 2, due_at: Some(_), .. })
    ));
    assert!(screen(&mut app, AgentId(0)).contains("next try in 12s"));
    // Past deadline: due now, without claiming the attempt started.
    let past = chrono::Utc::now().timestamp_millis() - 5_000;
    lifecycle(&mut app, "retry-progress", retry(3, past), serde_json::json!({}));
    assert!(matches!(
        test_agent(&app, AgentId(0)).resolve_turn_activity(),
        Some(TurnActivity::Retrying { attempt: 3, .. })
    ));
    assert!(screen(&mut app, AgentId(0)).contains("attempt due"));
    // A late, lower attempt must not regress the newer live phase.
    lifecycle(&mut app, "retry-progress", retry(2, past), serde_json::json!({}));
    assert!(matches!(
        test_agent(&app, AgentId(0)).resolve_turn_activity(),
        Some(TurnActivity::Retrying { attempt: 3, .. })
    ));
    // Replayed retry history never restarts the live phase.
    app.agents.get_mut(&AgentId(0)).unwrap().arm_late_replay_grace();
    lifecycle(&mut app, "retry-progress", retry(4, soon), serde_json::json!({ "isReplay": true }));
    assert!(matches!(
        test_agent(&app, AgentId(0)).resolve_turn_activity(),
        Some(TurnActivity::Retrying { attempt: 3, .. })
    ));
    // Real turn end still clears the phase.
    crate::app::dispatch::dispatch(Action::TaskComplete(TaskResult::PromptResponse {
        agent_id: AgentId(0), prompt_id: Some(prompt), http_status: None,
        result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
    }), &mut app);
    assert_eq!(test_agent(&app, AgentId(0)).resolve_turn_activity(), None);
}
