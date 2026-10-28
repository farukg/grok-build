use super::*;
use crate::app::agent_view::load_continuation::LoadDelivery;
use crate::acp::tracker::TurnActivity;
use crate::scrollback::types::ToolCompletion;
use xai_grok_shell::extensions::notification::{CompactPhase, RetryState, SessionNotification, SessionUpdate as XaiSessionUpdate};
use xai_grok_shell::session::acp_types::{AutomaticCompactInvocation, CompactInvocationId};
use xai_grok_shell::extensions::sampling_lifecycle::{SamplingLifecycle, SamplingOwner, SamplingPhase, RequestAttemptId};
use xai_acp_lib::{AcpArgs, AcpClientMessage};
use std::time::Duration;

fn acp_delivery(app: &mut AppView, id: AgentId, sid: &str, delivery: LoadDelivery, update: acp::SessionUpdate, prompt: &str) {
    assert_eq!(expect_agent(app, id).session.session_id.as_ref().unwrap().0.as_ref(), sid);
    let replay = matches!(delivery, LoadDelivery::Historical);
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    let msg = AcpClientMessage::SessionNotification(AcpArgs {
        request: acp::SessionNotification::new(acp::SessionId::new(sid), update)
            .meta(Some(serde_json::json!({"isReplay": replay, "promptId": prompt}).as_object().unwrap().clone())),
        response_tx,
    });
    crate::app::acp_handler::handle(msg, app);
}

fn lifecycle(app: &mut AppView, id: AgentId, sid: &str, delivery: LoadDelivery, update: XaiSessionUpdate, mut meta: serde_json::Value) {
    assert_eq!(expect_agent(app, id).session.session_id.as_ref().unwrap().0.as_ref(), sid);
    meta.as_object_mut().unwrap().insert("isReplay".into(), matches!(delivery, LoadDelivery::Historical).into());
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    let msg = AcpClientMessage::ExtNotification(AcpArgs {
        request: xai_grok_test_support::acp_fixtures::ext_notification("x.ai/session_notification", &SessionNotification {
            session_id: acp::SessionId::new(sid), update, meta: Some(meta),
        }),
        response_tx,
    });
    crate::app::acp_handler::handle(msg, app);
}

fn loaded(app: &mut AppView, id: AgentId, sid: &str, running: Option<&str>) {
    dispatch(Action::TaskComplete(TaskResult::SessionLoaded {
        agent_id: id, session_id: acp::SessionId::new(sid), models: None, modes: None,
        code_restored: false, restore_summary: None, restore_degree: None,
        running_prompt_id: running.map(str::to_owned),
    }), app);
}

fn tool(name: &str) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCall(acp::ToolCall::new(name.to_owned(), name.to_owned())
        .kind(acp::ToolKind::Read).status(acp::ToolCallStatus::InProgress))
}

fn complete_tool(name: &str) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(name.to_owned(),
        acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed))))
}

fn text(thinking: bool) -> acp::SessionUpdate {
    let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new("actual output")));
    if thinking { acp::SessionUpdate::AgentThoughtChunk(chunk) } else { acp::SessionUpdate::AgentMessageChunk(chunk) }
}

fn terminal(prompt: &str) -> XaiSessionUpdate {
    XaiSessionUpdate::TurnCompleted {
        prompt_id: prompt.into(), stop_reason: "end_turn".into(), agent_result: None,
        error_kind: None, usage: None, elapsed_ms: Some(4000),
    }
}

fn fresh() -> AppView {
    let mut app = test_app();
    dispatch(Action::LoadSession("load-owner".into(), None, false), &mut app);
    app
}

#[test]
fn loaded_task_preserves_live_tool_thinking_hook_and_exact_anchors() {
    let mut app = fresh();
    let id = AgentId(0);
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Historical, tool("historical-tool"), "old-prompt");
    let historical = expect_agent(&app, id).session.tracker.pending_tool_entry_id("historical-tool").unwrap();
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, tool("live-tool"), "live-prompt");
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, text(true), "live-prompt");
    let thinking = {
        let sb = &expect_agent(&app, id).scrollback;
        (0..sb.len()).filter_map(|i| sb.get(i)).find(|entry| matches!(entry.block, RenderBlock::Thinking(_))).unwrap().id
    };
    let entry = expect_agent(&app, id).session.tracker.pending_tool_entry_id("live-tool").unwrap();
    let anchor = Instant::now() - Duration::from_secs(30);
    {
        let view = app.agents.get_mut(&id).unwrap();
        view.turn_started_at = Some(anchor);
        view.turn_start_ms = Some(1000);
        view.turn_start_ms_prompt = Some("live-prompt".into());
    }
    lifecycle(&mut app, id, "load-owner", LoadDelivery::Live, XaiSessionUpdate::HookRunStarted {
        event_name: "pre_tool_use".into(), tool_name: Some("read_file".into()),
        prompt_id: Some("live-prompt".into()), count: 1,
    }, serde_json::json!({"promptId": "live-prompt"}));
    let hook = expect_agent(&app, id).session.tracker.hooks_running_since();
    assert!(hook.is_some());
    loaded(&mut app, id, "load-owner", Some("live-prompt"));
    let view = expect_agent(&app, id);
    assert_eq!(view.session.tracker.pending_tool_entry_id("live-tool"), Some(entry));
    assert!(view.scrollback.get_by_id(entry).unwrap().is_running);
    assert!(view.scrollback.get_by_id(thinking).unwrap().is_running);
    assert!(!view.scrollback.get_by_id(historical).unwrap().is_running);
    assert!(!view.scrollback.in_batch());
    assert_eq!(view.session.tracker.hooks_running_since(), hook);
    assert_eq!(view.turn_started_at, Some(anchor));
    assert_eq!(view.turn_start_ms, Some(1000));
    assert_eq!(view.turn_start_ms_prompt.as_deref(), Some("live-prompt"));
    assert_eq!(view.current_action_started_at(), None, "legacy hook delivery has no native phase clock");
    assert!(view.session.state.is_turn_running());
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, complete_tool("live-tool"), "live-prompt");
    assert_eq!(expect_agent(&app, id).scrollback.get_by_id(entry).unwrap().tool_completion(), ToolCompletion::Completed);
    lifecycle(&mut app, id, "load-owner", LoadDelivery::Live, terminal("live-prompt"), serde_json::json!({"promptId": "live-prompt"}));
    assert!(expect_agent(&app, id).session.state.is_idle());
    assert_eq!(expect_agent(&app, id).turn_started_at, None);
}

#[test]
fn live_terminal_before_loaded_task_cannot_be_resurrected_by_running_prompt_metadata() {
    let mut app = fresh();
    let id = AgentId(0);
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, tool("settled-tool"), "settled-prompt");
    let entry = expect_agent(&app, id).session.tracker.pending_tool_entry_id("settled-tool").unwrap();
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, complete_tool("settled-tool"), "settled-prompt");
    lifecycle(&mut app, id, "load-owner", LoadDelivery::Live, terminal("settled-prompt"), serde_json::json!({"promptId": "settled-prompt"}));
    let activity = expect_agent(&app, id).last_active_at;
    loaded(&mut app, id, "load-owner", Some("settled-prompt"));
    loaded(&mut app, id, "load-owner", Some("settled-prompt"));
    let view = expect_agent(&app, id);
    assert!(view.session.state.is_idle());
    assert_eq!(view.turn_started_at, None);
    assert_eq!(view.last_active_at, activity);
    assert_eq!(view.scrollback.get_by_id(entry).unwrap().tool_completion(), ToolCompletion::Completed);
    assert!(!view.scrollback.get_by_id(entry).unwrap().is_running);
}

#[test]
fn retry_and_automatic_compaction_live_owners_survive_load_completion_and_settle_only_on_terminal() {
    let id = AgentId(0);
    let mut retry = fresh();
    acp_delivery(&mut retry, id, "load-owner", LoadDelivery::Historical, text(true), "old");
    acp_delivery(&mut retry, id, "load-owner", LoadDelivery::Live, text(false), "retry-prompt");
    let scheduled = SamplingLifecycle {
        owner: SamplingOwner { prompt_id: "retry-prompt".into(), request_id: "request-a".to_owned().into(), attempt: RequestAttemptId(2) },
        phase: SamplingPhase::RetryScheduled { deadline: serde_json::from_value(serde_json::json!({
            "clock": {"clock": "unavailable", "reason": "unsupported_platform"}, "wall_deadline_ms": null,
        })).unwrap() },
    };
    lifecycle(&mut retry, id, "load-owner", LoadDelivery::Live, XaiSessionUpdate::RetryState(RetryState::Retrying {
        attempt: 2, max_retries: 4, reason: "transport".into(), error_type: None, next_attempt_at_ms: None,
    }), serde_json::json!({"promptId": "retry-prompt", "requestId": "request-a", "streamEpoch": 2,
        "samplingOwner": scheduled.owner, "samplingLifecycle": scheduled}));
    let projection = expect_agent(&retry, id).session.tracker.retry_activity().cloned();
    assert!(projection.is_some());
    lifecycle(&mut retry, id, "load-owner", LoadDelivery::Historical, XaiSessionUpdate::RetryState(RetryState::Exhausted {
        attempts: 9, reason: "old request".into(), is_rate_limited: true,
    }), serde_json::json!({"isReplay": true, "promptId": "old"}));
    loaded(&mut retry, id, "load-owner", Some("retry-prompt"));
    assert_eq!(expect_agent(&retry, id).session.tracker.retry_activity(), projection.as_ref());
    assert!(!expect_agent(&retry, id).session.rate_limited);
    lifecycle(&mut retry, id, "load-owner", LoadDelivery::Live, terminal("retry-prompt"), serde_json::json!({"promptId": "retry-prompt"}));
    assert!(expect_agent(&retry, id).session.tracker.retry_activity().is_none());

    let mut compact = fresh();
    acp_delivery(&mut compact, id, "load-owner", LoadDelivery::Live, text(false), "compact-prompt");
    let invocation = AutomaticCompactInvocation {
        id: CompactInvocationId(uuid::Uuid::from_u128(901)), prompt_id: Some("compact-prompt".into()),
    };
    lifecycle(&mut compact, id, "load-owner", LoadDelivery::Live, XaiSessionUpdate::AutoCompactStarted {
        phase: CompactPhase::Executing, tokens_used: 90_000, context_window: 100_000,
        percentage: 90, reason: "threshold".into(),
    }, serde_json::json!({"automaticCompactInvocation": invocation, "promptId": "compact-prompt", "eventId": "load-owner-1"}));
    assert!(expect_agent(&compact, id).session.tracker.has_correlated_compaction());
    loaded(&mut compact, id, "load-owner", Some("compact-prompt"));
    assert!(expect_agent(&compact, id).session.tracker.has_correlated_compaction());
    assert_eq!(expect_agent(&compact, id).session.tracker.activity(), Some(TurnActivity::AutoCompacting));
    lifecycle(&mut compact, id, "load-owner", LoadDelivery::Live, XaiSessionUpdate::AutoCompactCompleted {
        tokens_before: Some(90_000), tokens_after: 20_000, elapsed_ms: Some(250), summary_preview: None,
    }, serde_json::json!({"automaticCompactInvocation": invocation, "promptId": "compact-prompt", "eventId": "load-owner-2"}));
    assert!(!matches!(expect_agent(&compact, id).session.tracker.activity(), Some(TurnActivity::AutoCompacting)));
}

#[test]
fn prompt_rpc_terminal_before_loaded_task_keeps_the_real_tool_outcome_settled() {
    let mut app = fresh();
    let id = AgentId(0);
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, tool("rpc-tool"), "rpc-prompt");
    let entry = expect_agent(&app, id).session.tracker.pending_tool_entry_id("rpc-tool").unwrap();
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, complete_tool("rpc-tool"), "rpc-prompt");
    dispatch(Action::TaskComplete(TaskResult::PromptResponse {
        agent_id: id, prompt_id: Some("rpc-prompt".into()), http_status: None,
        result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
    }), &mut app);
    let activity = expect_agent(&app, id).last_active_at;
    loaded(&mut app, id, "load-owner", Some("rpc-prompt"));
    assert!(expect_agent(&app, id).session.state.is_idle());
    assert_eq!(expect_agent(&app, id).last_active_at, activity);
    assert_eq!(expect_agent(&app, id).scrollback.get_by_id(entry).unwrap().tool_completion(), ToolCompletion::Completed);
}

#[derive(Clone, Copy)]
enum ReconnectResult { Cursor, Replay, Failed }

#[test]
fn reconnect_cursor_full_replay_and_failure_stash_preserve_live_owner_and_reject_old_generation() {
    for outcome in [ReconnectResult::Cursor, ReconnectResult::Replay, ReconnectResult::Failed] {
        let mut app = fresh();
        let id = AgentId(0);
        loaded(&mut app, id, "load-owner", None);
        let stash_entry = app.agents.get_mut(&id).unwrap().scrollback.push_block(RenderBlock::system("pre-outage history"));
        app.agents.get_mut(&id).unwrap().begin_session_reload(71);
        if matches!(outcome, ReconnectResult::Replay | ReconnectResult::Failed) {
            acp_delivery(&mut app, id, "load-owner", LoadDelivery::Historical, text(false), "old-prompt");
        }
        acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, tool("reconnect-tool"), "reconnect-prompt");
        let entry = expect_agent(&app, id).session.tracker.pending_tool_entry_id("reconnect-tool").unwrap();
        let anchor = Instant::now() - Duration::from_secs(50);
        app.agents.get_mut(&id).unwrap().turn_started_at = Some(anchor);
        assert!(!app.agents.get_mut(&id).unwrap().finish_session_reload(70, true));
        assert_eq!(expect_agent(&app, id).session.tracker.pending_tool_entry_id("reconnect-tool"), Some(entry));
        assert!(app.agents.get_mut(&id).unwrap().finalize_reload_and_maybe_adopt(
            71, !matches!(outcome, ReconnectResult::Failed), Some("reconnect-prompt".into()),
        ));
        let view = expect_agent(&app, id);
        assert_eq!(view.session.tracker.pending_tool_entry_id("reconnect-tool"), Some(entry));
        assert!(view.scrollback.get_by_id(entry).unwrap().is_running);
        assert_eq!(view.turn_started_at, Some(anchor));
        assert_eq!(view.scrollback.get_by_id(stash_entry).is_some(), !matches!(outcome, ReconnectResult::Replay));
        acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, complete_tool("reconnect-tool"), "reconnect-prompt");
        assert_eq!(expect_agent(&app, id).scrollback.get_by_id(entry).unwrap().tool_completion(), ToolCompletion::Completed);
    }
}

#[test]
fn reconnect_supersession_and_old_binding_finalize_do_not_finish_the_live_owner() {
    let mut app = fresh();
    let id = AgentId(0);
    loaded(&mut app, id, "load-owner", None);
    app.agents.get_mut(&id).unwrap().begin_session_reload(81);
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Historical, text(false), "old");
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, tool("generation-tool"), "generation-prompt");
    let entry = expect_agent(&app, id).session.tracker.pending_tool_entry_id("generation-tool").unwrap();
    let anchor = Instant::now() - Duration::from_secs(20);
    app.agents.get_mut(&id).unwrap().turn_started_at = Some(anchor);
    app.agents.get_mut(&id).unwrap().supersede_session_reload(81, 82);
    app.agents.get_mut(&id).unwrap().begin_session_reload(82);
    assert!(!app.agents.get_mut(&id).unwrap().finish_session_reload(81, true));
    assert!(app.agents.get_mut(&id).unwrap().finalize_reload_and_maybe_adopt(82, true, Some("generation-prompt".into())));
    assert_eq!(expect_agent(&app, id).turn_started_at, Some(anchor));
    assert_eq!(expect_agent(&app, id).session.tracker.pending_tool_entry_id("generation-tool"), Some(entry));
    assert!(expect_agent(&app, id).scrollback.get_by_id(entry).unwrap().is_running);

    let mut rebound = fresh();
    loaded(&mut rebound, id, "load-owner", None);
    rebound.agents.get_mut(&id).unwrap().begin_session_reload(91);
    rebound.agents.get_mut(&id).unwrap().bind_session_id(acp::SessionId::new("new-binding"));
    acp_delivery(&mut rebound, id, "new-binding", LoadDelivery::Live, tool("binding-tool"), "new-prompt");
    let entry = expect_agent(&rebound, id).session.tracker.pending_tool_entry_id("binding-tool").unwrap();
    assert!(!rebound.agents.get_mut(&id).unwrap().finalize_reload_and_maybe_adopt(91, true, Some("old-prompt".into())));
    assert_eq!(expect_agent(&rebound, id).session.current_prompt_id.as_deref(), Some("new-prompt"));
    assert_eq!(expect_agent(&rebound, id).session.tracker.pending_tool_entry_id("binding-tool"), Some(entry));
    assert!(expect_agent(&rebound, id).scrollback.get_by_id(entry).unwrap().is_running);
}

#[test]
fn reconnect_terminal_before_finalize_remains_settled_for_replay_cursor_and_failure() {
    for outcome in [ReconnectResult::Cursor, ReconnectResult::Replay, ReconnectResult::Failed] {
        let mut app = fresh();
        let id = AgentId(0);
        loaded(&mut app, id, "load-owner", None);
        app.agents.get_mut(&id).unwrap().begin_session_reload(72);
        if matches!(outcome, ReconnectResult::Replay | ReconnectResult::Failed) {
            acp_delivery(&mut app, id, "load-owner", LoadDelivery::Historical, text(false), "old-prompt");
        }
        acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, tool("done-reconnect"), "done-prompt");
        let entry = expect_agent(&app, id).session.tracker.pending_tool_entry_id("done-reconnect").unwrap();
        acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, complete_tool("done-reconnect"), "done-prompt");
        lifecycle(&mut app, id, "load-owner", LoadDelivery::Live, terminal("done-prompt"), serde_json::json!({"promptId": "done-prompt"}));
        assert!(app.agents.get_mut(&id).unwrap().finalize_reload_and_maybe_adopt(
            72, !matches!(outcome, ReconnectResult::Failed), Some("done-prompt".into()),
        ));
        let view = expect_agent(&app, id);
        assert!(view.session.state.is_idle());
        assert!(view.turn_started_at.is_none());
        assert!(view.session.tracker.pending_tool_entry_id("done-reconnect").is_none());
        assert_eq!(view.scrollback.get_by_id(entry).unwrap().tool_completion(), ToolCompletion::Completed);
    }
}

#[test]
fn load_failure_keeps_real_live_binding_but_foreign_binding_results_cannot_clear_it() {
    let mut app = fresh();
    let id = AgentId(0);
    acp_delivery(&mut app, id, "load-owner", LoadDelivery::Live, tool("failure-tool"), "failure-prompt");
    let entry = expect_agent(&app, id).session.tracker.pending_tool_entry_id("failure-tool").unwrap();
    for sid in ["foreign-session", "load-owner"] {
        dispatch(Action::TaskComplete(TaskResult::SessionLoadFailed {
            agent_id: id, session_id: acp::SessionId::new(sid), error: "load transport failed".into(),
        }), &mut app);
        let view = expect_agent(&app, id);
        assert_eq!(view.session.session_id.as_ref().unwrap().0.as_ref(), "load-owner");
        assert_eq!(view.session.tracker.pending_tool_entry_id("failure-tool"), Some(entry));
        assert!(view.scrollback.get_by_id(entry).unwrap().is_running);
        assert!(view.session.state.is_turn_running());
    }
}

#[derive(Clone, Copy)]
enum LoadRoute { Fresh, Cursor, Replay, Failed }

#[test]
fn inactive_child_and_wake_continuations_survive_load_without_focus_or_clock_restart() {
    let id = AgentId(1);
    for (route, child) in [LoadRoute::Fresh, LoadRoute::Cursor, LoadRoute::Replay, LoadRoute::Failed]
        .into_iter().flat_map(|route| [false, true].map(|child| (route, child)))
    {
        let mut app = fresh();
        loaded(&mut app, AgentId(0), "load-owner", None);
        let mut view = crate::app::agent_view::test_fixtures::make_agent();
        view.session.id = id;
        view.bind_session_id(acp::SessionId::new("inactive-owner"));
        if child {
            crate::app::session_views::test_support::link_child(&mut app.agents, AgentId(0), id, view, Instant::now());
        } else {
            app.agents.insert(id, view);
        }
        match route {
            LoadRoute::Fresh => app.agents.get_mut(&id).unwrap().begin_replay_window(),
            LoadRoute::Cursor | LoadRoute::Replay | LoadRoute::Failed => app.agents.get_mut(&id).unwrap().begin_session_reload(99),
        }
        if matches!(route, LoadRoute::Replay | LoadRoute::Failed) {
            acp_delivery(&mut app, id, "inactive-owner", LoadDelivery::Historical, text(false), "old-prompt");
        }
        app.active_view = ActiveView::Agent(AgentId(0));
        let prompt = if child { "child-prompt" } else { "task-completed-load-continuation" };
        acp_delivery(&mut app, id, "inactive-owner", LoadDelivery::Live, text(false), prompt);
        acp_delivery(&mut app, id, "inactive-owner", LoadDelivery::Live, tool("inactive-tool"), prompt);
        let anchor = Instant::now() - Duration::from_secs(60);
        app.agents.get_mut(&id).unwrap().turn_started_at = Some(anchor);
        let entry = expect_agent(&app, id).session.tracker.pending_tool_entry_id("inactive-tool").unwrap();
        if !child { assert!(expect_agent(&app, id).wake_turn_active()); }
        match route {
            LoadRoute::Fresh => loaded(&mut app, id, "inactive-owner", Some(prompt)),
            LoadRoute::Cursor | LoadRoute::Replay | LoadRoute::Failed => {
                assert!(app.agents.get_mut(&id).unwrap().finalize_reload_and_maybe_adopt(
                    99, !matches!(route, LoadRoute::Failed), Some(prompt.into()),
                ));
            }
        }
        assert_eq!(expect_agent(&app, id).session.tracker.pending_tool_entry_id("inactive-tool"), Some(entry));
        assert_eq!(expect_agent(&app, id).turn_started_at, Some(anchor));
        assert!(matches!(app.active_view, ActiveView::Agent(AgentId(0))));
        app.active_view = ActiveView::Agent(id);
        assert_eq!(expect_agent(&app, id).turn_started_at, Some(anchor));
        assert!(expect_agent(&app, id).scrollback.get_by_id(entry).unwrap().is_running);
        lifecycle(&mut app, id, "inactive-owner", LoadDelivery::Live, terminal(prompt), serde_json::json!({"promptId": prompt}));
        loaded(&mut app, id, "inactive-owner", Some(prompt));
        assert!(!expect_agent(&app, id).wake_turn_active());
        assert!(expect_agent(&app, id).session.state.is_idle());
    }
}
