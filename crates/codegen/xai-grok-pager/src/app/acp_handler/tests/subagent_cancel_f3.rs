use super::*;
use crate::app::actions::{Action, SubagentKillOutcome, TaskResult};
use crate::app::dispatch::dispatch;
use xai_grok_shell::extensions::task::SubagentTerminalDto;
use xai_grok_tools::implementations::grok_build::task::terminal::{
    SubagentTerminal, SubagentTerminalState,
};

fn terminal(state: SubagentTerminalState) -> SubagentTerminalDto {
    SubagentTerminalDto {
        subagent_id: "child-1".into(),
        parent_session_id: "sess-1".into(),
        child_session_id: "child-1".into(),
        result: SubagentTerminal {
            attempt_id: "at1.one".into(),
            state,
            output: "producer output".into(),
            tool_calls: 17,
            turns: 4,
            duration_ms: 91_234,
            tokens_used: 8_765,
        },
    }
}

fn complete(app: &mut AppView, binding_epoch: u32, outcome: SubagentKillOutcome) {
    dispatch(Action::TaskComplete(TaskResult::KillSubagentComplete {
        binding_epoch,
        session_id: "sess-1".into(),
        subagent_id: "child-1".into(),
        attempt_id: Some("at1.one".into()),
        outcome,
    }), app);
}

#[test]
fn cancel_refresh_applies_exact_terminal_evidence_through_appview() {
    for (state, status, error) in [
        (SubagentTerminalState::Completed, "completed", None),
        (SubagentTerminalState::Failed { message: "producer failure".into() }, "failed", Some("producer failure")),
        (SubagentTerminalState::Interrupted {
            cause: xai_grok_tools::implementations::grok_build::task::types::InterruptionCause::ExplicitStop {
                actor: xai_grok_tools::implementations::grok_build::task::types::SubagentActor::Human,
            },
        }, "cancelled", Some("Subagent was stopped by human user.")),
    ] {
        let mut app = make_app_with_agent("sess-1");
        assert!(handle(make_ext_session_notification(
            "sess-1", test_subagent_spawned("sess-1", "child-1"),
        ), &mut app));
        dispatch(Action::KillSubagent("child-1".into()), &mut app);
        complete(&mut app, 0, SubagentKillOutcome::Terminal(terminal(state)));
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        draw_view(agent);
        let info = &agent.subagent_sessions["child-1"];
        assert!(info.is_finished());
        assert_eq!(info.attempt.status.as_deref(), Some(status));
        assert_eq!(info.attempt.error.as_deref(), error);
        assert_eq!(info.attempt.tool_calls, Some(17));
        assert_eq!(info.attempt.turns, Some(4));
        assert_eq!(info.attempt.duration_ms, Some(91_234));
        assert_eq!(info.attempt.tokens_used, Some(8_765));
        assert!(!info.attempt.pending_kill);
        let row = agent.scrollback.get_by_id(info.attempt.scrollback_entry_id.unwrap()).unwrap();
        let RenderBlock::Subagent(block) = &row.block else { panic!("subagent row") };
        assert!(!matches!(block.kind, crate::scrollback::blocks::SubagentBlockKind::Started));
    }
}

#[test]
fn cancel_refresh_cannot_overwrite_a_real_terminal() {
    let mut app = make_app_with_agent("sess-1");
    assert!(handle(make_ext_session_notification(
        "sess-1", test_subagent_spawned("sess-1", "child-1"),
    ), &mut app));
    assert!(handle(make_ext_session_notification(
        "sess-1", test_subagent_finished("child-1"),
    ), &mut app));
    let before = app.agents.get(&AgentId(0)).unwrap().scrollback.len();
    complete(&mut app, 0, SubagentKillOutcome::Terminal(terminal(
        SubagentTerminalState::Failed { message: "late response".into() },
    )));
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    draw_view(agent);
    let info = &agent.subagent_sessions["child-1"];
    assert_eq!(info.attempt.status.as_deref(), Some("completed"));
    assert_eq!(info.attempt.tool_calls, Some(2));
    assert_eq!(info.attempt.turns, Some(1));
    assert_eq!(info.attempt.duration_ms, Some(500));
    assert_eq!(agent.scrollback.len(), before);
}

#[test]
fn accepted_cancel_waits_for_the_actual_terminal_owner() {
    let mut app = make_app_with_agent("sess-1");
    assert!(handle(make_ext_session_notification(
        "sess-1", test_subagent_spawned("sess-1", "child-1"),
    ), &mut app));
    dispatch(Action::KillSubagent("child-1".into()), &mut app);
    complete(&mut app, 0, SubagentKillOutcome::StoppedLive);
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    draw_view(agent);
    let info = &agent.subagent_sessions["child-1"];
    assert!(info.is_running());
    assert!(info.attempt.pending_kill);
    assert_eq!(info.attempt.status, None);
    assert_eq!(info.attempt.duration_ms, None);
    assert_eq!(info.attempt.tool_calls, None);
    assert!(handle(make_ext_session_notification(
        "sess-1", test_subagent_finished("child-1"),
    ), &mut app));
    let info = &app.agents.get(&AgentId(0)).unwrap().subagent_sessions["child-1"];
    assert!(info.is_finished());
    assert_eq!(info.attempt.tool_calls, Some(2));
    assert_eq!(info.attempt.duration_ms, Some(500));
}

#[test]
fn cancel_terminal_for_a_different_attempt_cannot_finish_the_row() {
    let mut app = make_app_with_agent("sess-1");
    assert!(handle(make_ext_session_notification(
        "sess-1", test_subagent_spawned("sess-1", "child-1"),
    ), &mut app));
    let mut evidence = terminal(SubagentTerminalState::Completed);
    evidence.result.attempt_id = "at1.old".into();
    complete(&mut app, 0, SubagentKillOutcome::Terminal(evidence));
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    draw_view(agent);
    let info = &agent.subagent_sessions["child-1"];
    assert!(info.is_running());
    assert_eq!(info.attempt.status, None);
    assert_eq!(info.attempt.tool_calls, None);
    assert_eq!(info.attempt.duration_ms, None);
}

#[test]
fn cancel_reply_from_an_old_binding_leaves_the_current_request_untouched() {
    let mut app = make_app_with_agent("sess-1");
    assert!(handle(make_ext_session_notification(
        "sess-1", test_subagent_spawned("sess-1", "child-1"),
    ), &mut app));
    dispatch(Action::KillSubagent("child-1".into()), &mut app);
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.session_binding_epoch = 1;
    let requested = agent.subagent_sessions["child-1"].attempt.kill_requested_at;
    complete(&mut app, 0, SubagentKillOutcome::NotFound);
    complete(&mut app, 0, SubagentKillOutcome::Terminal(terminal(SubagentTerminalState::Completed)));
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    draw_view(agent);
    let info = &agent.subagent_sessions["child-1"];
    assert!(info.is_running());
    assert!(info.attempt.pending_kill);
    assert_eq!(info.attempt.kill_requested_at, requested);
    assert_eq!(info.attempt.status, None);
    assert_eq!(info.attempt.duration_ms, None);
}
