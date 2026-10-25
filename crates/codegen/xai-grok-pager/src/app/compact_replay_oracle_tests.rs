use super::*;
use crate::acp::tracker::TurnActivity;
use crate::app::agent::{AgentCommand, InFlightPrompt};
use crate::app::app_view::AppView;
use crate::app::dispatch::tests::test_app_with_agent;
use crate::scrollback::EntryId;
use xai_grok_shell::extensions::notification::{CompactPhase, SessionNotification, SessionUpdate};

fn started(phase: CompactPhase) -> SessionUpdate {
    SessionUpdate::AutoCompactStarted {
        phase, tokens_used: 90_000, context_window: 131_072, percentage: 68, reason: "threshold".into(),
    }
}

fn completed() -> SessionUpdate {
    SessionUpdate::AutoCompactCompleted {
        tokens_before: Some(90_000), tokens_after: 40_000, elapsed_ms: None, summary_preview: None,
    }
}

fn notification(sid: &str, update: SessionUpdate, meta: Option<serde_json::Value>) -> SessionNotification {
    SessionNotification { session_id: acp::SessionId::new(sid), update, meta }
}

fn deliver(app: &mut AppView, method: &str, notification: &SessionNotification) {
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    crate::app::acp_handler::handle(xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
        request: xai_grok_test_support::acp_fixtures::ext_notification(method, notification), response_tx,
    }), app);
}

fn app_with_child(sid: &str, finished: bool) -> (AppView, AgentId) {
    let mut info = make_info();
    info.child_session_id = sid.into();
    info.set_finished_for_test(finished);
    let (views, child) = session_views(sid, make_min_child_view(), info);
    let mut app = test_app_with_agent();
    app.agents = views;
    (app, child)
}

fn context_fixture() -> xai_grok_shell::session::ContextInfo {
    xai_grok_shell::session::ContextInfo {
        used: 90_000, total: 131_072, system_prompt_tokens: 113, tool_definitions_count: 3,
        tool_definitions_tokens: 29, compaction_count: 7, turn_count: 11, tool_call_count: 13,
        message_count: 17, message_tokens: 19, free_tokens: 41_072, usage_pct: 68,
        ..Default::default()
    }
}

fn seed_automatic(app: &mut AppView, id: AgentId, sid: &str) -> (Instant, Instant) {
    let agent = app.agents.get_mut(&id).unwrap();
    agent.session.start_command(AgentCommand::Compact);
    let now = match agent.session.state {
        AgentState::CommandRunning { started_at, .. } => started_at,
        AgentState::Idle | AgentState::TurnRunning | AgentState::TurnCancelling
        | AgentState::CommandCancelling { .. } => unreachable!(),
    };
    agent.session.current_prompt_id = Some(format!("{sid}-current"));
    agent.session.in_flight_prompt = Some(InFlightPrompt {
        text: format!("{sid}-stash"), images: vec![], scrollback_entry: EntryId::new(1),
        combined_scrollback_entries: vec![], chip_elements: vec![],
    });
    agent.session.models.override_context_window(131_072);
    agent.turn_started_at = Some(now);
    agent.activity_started_at = Some(now);
    deliver(app, "x.ai/session_notification", &notification(sid, started(CompactPhase::Executing), None));
    let agent = app.agents.get_mut(&id).unwrap();
    agent.context_state = Some(context_fixture());
    agent.todo.update_todos(vec![xai_grok_shell::tools::TodoItem {
        content: format!("{sid}-todo"), priority: Default::default(),
        status: xai_grok_shell::tools::TodoStatus::Pending, meta: None,
    }]);
    (agent.turn_started_at.unwrap(), agent.activity_started_at.unwrap())
}

fn assert_automatic(app: &AppView, id: AgentId, sid: &str, timers: (Instant, Instant)) {
    let agent = app.agents.get(&id).unwrap();
    assert_eq!(agent.session.turn_activity(), Some(TurnActivity::AutoCompacting));
    assert_eq!(agent.turn_started_at, Some(timers.0));
    assert_eq!(agent.activity_started_at, Some(timers.1));
    assert!(matches!(agent.session.state, AgentState::CommandRunning {
        command: AgentCommand::Compact, started_at,
    } if started_at == timers.0));
    assert_eq!(agent.session.current_prompt_id.as_deref(), Some(format!("{sid}-current").as_str()));
    assert!(agent.session.in_flight_prompt.is_none());
    let stash = agent.session.compact_held_prompt.as_ref().unwrap();
    assert_eq!(stash.text, format!("{sid}-stash"));
    assert_eq!(stash.scrollback_entry, EntryId::new(1));
    assert!(stash.images.is_empty());
    assert!(stash.chip_elements.is_empty());
    assert!(stash.combined_scrollback_entries.is_empty());
    assert_eq!(agent.todo.todos().len(), 1);
    assert_eq!(agent.todo.todos()[0].content, format!("{sid}-todo"));
    assert!(matches!(agent.todo.todos()[0].status, xai_grok_shell::tools::TodoStatus::Pending));
    assert_eq!(serde_json::to_value(&agent.todo.todos()[0]).unwrap(), serde_json::json!({
        "content": format!("{sid}-todo"), "priority": "medium", "status": "pending",
    }));
    let context = agent.context_state.as_ref().unwrap();
    assert_eq!(context.used, 90_000);
    assert_eq!(context.total, 131_072);
    assert_eq!(serde_json::to_value(context).unwrap(), serde_json::to_value(context_fixture()).unwrap());
}

fn malformed_and_historical_ids() -> Vec<serde_json::Value> {
    vec![serde_json::json!("00000000-0000-0000-0000-000000000011"),
        serde_json::json!("00000000-0000-0000-0000-000000000012"), serde_json::Value::Null,
        serde_json::json!(false), serde_json::json!(17), serde_json::json!([]),
        serde_json::json!({}), serde_json::json!("invalid")]
}

#[test]
fn root_replay_and_child_live_routes_reject_tagged_lifecycle_before_all_side_effects() {
    let (mut app, child) = app_with_child("correlated-child", false);
    let root_timers = seed_automatic(&mut app, AgentId(0), "parent");
    let child_timers = seed_automatic(&mut app, child, "correlated-child");
    let root_len = app.agents.get(&AgentId(0)).unwrap().scrollback.len();
    let child_len = app.agents.get(&child).unwrap().scrollback.len();
    for (sid, method) in [("parent", "x.ai/session/update"), ("correlated-child", "x.ai/session_notification")] {
        for raw in malformed_and_historical_ids() {
            for update in [started(CompactPhase::Queued), started(CompactPhase::Executing), completed(),
                SessionUpdate::AutoCompactFailed { error: "old failure".into() },
                SessionUpdate::AutoCompactCancelled {
                    reason: xai_grok_shell::extensions::notification::AutoCompactCancelReason::UserCancelled,
                }] {
                deliver(&mut app, method, &notification(sid, update, Some(serde_json::json!({
                    "compactInvocationId": raw, "isReplay": true,
                }))));
                assert_automatic(&app, AgentId(0), "parent", root_timers);
                assert_automatic(&app, child, "correlated-child", child_timers);
                assert_eq!(app.agents.get(&AgentId(0)).unwrap().scrollback.len(), root_len);
                assert_eq!(app.agents.get(&child).unwrap().scrollback.len(), child_len);
            }
        }
    }
    app.agents.get_mut(&AgentId(0)).unwrap().arm_late_replay_grace();
    deliver(&mut app, "x.ai/session/update", &notification("parent", completed(), Some(serde_json::json!({"isReplay": true}))));
    let root = app.agents.get(&AgentId(0)).unwrap();
    assert_ne!(root.session.turn_activity(), Some(TurnActivity::AutoCompacting));
    assert!(root.session.compact_held_prompt.is_none());
    assert!(root.todo.todos().is_empty());
    assert_eq!(root.context_state.as_ref().unwrap().used, 40_000);
    assert!(matches!(root.session.state, AgentState::CommandRunning { command: AgentCommand::Compact, .. }));
    assert_automatic(&app, child, "correlated-child", child_timers);
    deliver(&mut app, "x.ai/session_notification", &notification("correlated-child", completed(), None));
    let child_view = app.agents.get(&child).unwrap();
    assert_ne!(child_view.session.turn_activity(), Some(TurnActivity::AutoCompacting));
    assert!(child_view.session.compact_held_prompt.is_none());
    assert_eq!(child_view.context_state.as_ref().unwrap().used, 40_000);
    assert!(matches!(child_view.session.state, AgentState::CommandRunning { command: AgentCommand::Compact, .. }));
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().context_state.as_ref().unwrap().used, 40_000);
}

fn write_history(home: &std::path::Path, sid: &str, with_acp: bool) {
    let dir = home.join("sessions").join(urlencoding::encode("/tmp").as_ref()).join(sid);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("summary.json"), "{}").unwrap();
    let mut lines = Vec::new();
    for raw in malformed_and_historical_ids() {
        for update in [started(CompactPhase::Queued), started(CompactPhase::Executing), completed(),
            SessionUpdate::AutoCompactFailed { error: "persisted old failure".into() },
            SessionUpdate::AutoCompactCancelled {
                reason: xai_grok_shell::extensions::notification::AutoCompactCancelReason::UserCancelled,
            }] {
            lines.push(serde_json::json!({"method": "x.ai/session_notification", "params":
                notification(sid, update, Some(serde_json::json!({"compactInvocationId": raw}))) }).to_string());
        }
    }
    if with_acp {
        lines.push(serde_json::json!({"method": "session/update", "params": {
            "sessionId": sid, "update": {"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "persisted child prompt"}},
        }}).to_string());
        lines.push(serde_json::json!({"method": "x.ai/session_notification", "params":
            notification(sid, started(CompactPhase::Executing), None) }).to_string());
        lines.push(serde_json::json!({"method": "x.ai/session_notification", "params":
            notification(sid, completed(), None) }).to_string());
    }
    std::fs::write(dir.join("updates.jsonl"), lines.join("\n") + "\n").unwrap();
}

fn assert_historical_markers(app: &AppView, child: AgentId) {
    let child = app.agents.get(&child).unwrap();
    let events: Vec<_> = (0..child.scrollback.len()).filter_map(|i| match &child.scrollback.entry(i)?.block {
        RenderBlock::SessionEvent(event) => Some(&event.event), _ => None,
    }).collect();
    assert_eq!(events.iter().filter(|event| matches!(event, crate::scrollback::blocks::SessionEvent::CompactionStarted { .. })).count(), 1);
    assert_eq!(events.iter().filter(|event| matches!(event, crate::scrollback::blocks::SessionEvent::CompactionCompleted { .. })).count(), 1);
    assert!(!events.iter().any(|event| matches!(event,
        crate::scrollback::blocks::SessionEvent::CompactCompleted { .. }
        | crate::scrollback::blocks::SessionEvent::CompactStarted
        | crate::scrollback::blocks::SessionEvent::CompactionFailed { .. }
        | crate::scrollback::blocks::SessionEvent::CompactionCancelled)),
        "historical tagged manual lifecycle does not restore RPC-only manual markers");
    assert!(child.session.state.is_idle());
    assert!(child.turn_started_at.is_none());
    assert!(child.activity_started_at.is_none());
    assert!(child.session.in_flight_prompt.is_none());
    assert!(child.session.compact_held_prompt.is_none());
    assert!(child.todo.todos().is_empty());
}

#[test]
fn child_eviction_replays_tagged_disk_history_without_reviving_manual_markers_or_touching_root() {
    let home = tempfile::tempdir().unwrap();
    write_history(home.path(), "disk-correlated-child", true);
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let (mut app, child) = app_with_child("disk-correlated-child", true);
    let timers = seed_automatic(&mut app, AgentId(0), "parent");
    assert_eq!(replay_on_open(&mut app.agents, child), ChildReplayOutcome::Replayed);
    assert_historical_markers(&app, child);
    assert_automatic(&app, AgentId(0), "parent", timers);
    assert_eq!(evict_on_leave(&mut app.agents, child), EvictOutcome::Evicted);
    assert!(app.agents.get(&child).unwrap().scrollback.is_empty());
    assert_eq!(replay_on_open(&mut app.agents, child), ChildReplayOutcome::Replayed);
    assert_historical_markers(&app, child);
    assert_automatic(&app, AgentId(0), "parent", timers);
    set_replay_grok_home_for_tests(None);
}

#[test]
fn metadata_without_a_correlation_key_still_applies_untagged_automatic_completion() {
    for meta in [serde_json::Value::Null, serde_json::json!(false), serde_json::json!(17),
        serde_json::json!([]), serde_json::json!("invalid"), serde_json::json!({"future": true})] {
        let (mut app, child) = app_with_child("untagged-child", false);
        seed_automatic(&mut app, AgentId(0), "parent");
        let child_timers = seed_automatic(&mut app, child, "untagged-child");
        deliver(&mut app, "x.ai/session_notification", &notification("parent", completed(), Some(meta)));
        let root = app.agents.get(&AgentId(0)).unwrap();
        assert_ne!(root.session.turn_activity(), Some(TurnActivity::AutoCompacting));
        assert!(root.session.compact_held_prompt.is_none());
        assert!(root.todo.todos().is_empty());
        assert_eq!(root.context_state.as_ref().unwrap().used, 40_000);
        assert!(matches!(root.session.state, AgentState::CommandRunning { command: AgentCommand::Compact, .. }));
        assert_automatic(&app, child, "untagged-child", child_timers);
    }
}

#[test]
fn tagged_disk_replay_preserves_newer_child_and_root_automatic_owners() {
    let home = tempfile::tempdir().unwrap();
    let sid = "newer-automatic-child";
    write_history(home.path(), sid, false);
    write_history(home.path(), "parent", false);
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let (mut app, child) = app_with_child(sid, true);
    let root_timers = seed_automatic(&mut app, AgentId(0), "parent");
    let child_timers = seed_automatic(&mut app, child, sid);
    let root_len = app.agents.get(&AgentId(0)).unwrap().scrollback.len();
    let child_len = app.agents.get(&child).unwrap().scrollback.len();
    assert_eq!(replay_on_open(&mut app.agents, child), ChildReplayOutcome::FoundNothingOnDisk);
    assert_automatic(&app, AgentId(0), "parent", root_timers);
    assert_automatic(&app, child, sid, child_timers);
    assert_eq!(app.agents.get(&child).unwrap().scrollback.len(), child_len);
    assert_eq!(evict_on_leave(&mut app.agents, child), EvictOutcome::Retained);
    let path = home.path().join("sessions").join(urlencoding::encode("/tmp").as_ref())
        .join("parent").join("updates.jsonl");
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let envelope: serde_json::Value = serde_json::from_str(line).unwrap();
        let notification: SessionNotification = serde_json::from_value(envelope["params"].clone()).unwrap();
        deliver(&mut app, "x.ai/session/update", &notification);
        assert_automatic(&app, AgentId(0), "parent", root_timers);
        assert_automatic(&app, child, sid, child_timers);
        assert_eq!(app.agents.get(&AgentId(0)).unwrap().scrollback.len(), root_len);
        assert_eq!(app.agents.get(&child).unwrap().scrollback.len(), child_len);
    }
    set_replay_grok_home_for_tests(None);
}

#[test]
fn tagged_xai_only_disk_history_cannot_authorize_child_eviction() {
    let home = tempfile::tempdir().unwrap();
    write_history(home.path(), "xai-only-correlated-child", false);
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let (mut app, child) = app_with_child("xai-only-correlated-child", true);
    app.agents.get_mut(&child).unwrap().scrollback.push_block(RenderBlock::user_prompt("only retained copy"));
    assert_eq!(evict_on_leave(&mut app.agents, child), EvictOutcome::Retained);
    assert_eq!(replay_on_open(&mut app.agents, child), ChildReplayOutcome::FoundNothingOnDisk);
    assert_eq!(app.agents.get(&child).unwrap().scrollback.len(), 1);
    assert!(matches!(app.agents.get(&child).unwrap().scrollback.entry(0).unwrap().block, RenderBlock::UserPrompt(_)));
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().subagent_sessions["xai-only-correlated-child"].transcript,
        ChildTranscript::MemoryOnly);
    assert_eq!(evict_on_leave(&mut app.agents, child), EvictOutcome::Retained);
    set_replay_grok_home_for_tests(None);
}
