use super::*;

pub(crate) fn apply_cancel_terminal(
    app: &mut AppView,
    session_id: &acp::SessionId,
    subagent_id: &str,
    attempt_id: Option<&str>,
    terminal: xai_grok_shell::extensions::task::SubagentTerminalDto,
) -> bool {
    use xai_grok_tools::implementations::grok_build::task::terminal::SubagentTerminalState;

    if terminal.parent_session_id != session_id.0.as_ref()
        || terminal.subagent_id != subagent_id
        || Some(terminal.result.attempt_id.as_str()) != attempt_id
    {
        return false;
    }
    let current = app.agents.values().any(|agent| {
        agent.session.session_id.as_ref() == Some(session_id)
            && agent.subagent_sessions.get(&terminal.child_session_id).is_some_and(|info| {
                info.subagent_id.as_ref() == subagent_id
                    && info.attempt.lifecycle.current_attempt_id() == attempt_id
                    && !info.is_finished()
            })
    });
    if !current {
        return false;
    }
    let (status, error) = match terminal.result.state {
        SubagentTerminalState::Completed => ("completed", None),
        SubagentTerminalState::Failed { message } => ("failed", Some(message)),
        SubagentTerminalState::Interrupted { cause } => ("cancelled", Some(cause.model_text())),
    };
    let child_session_id = terminal.child_session_id.clone();
    let notification = SessionNotification {
        session_id: session_id.clone(),
        update: XaiSessionUpdate::SubagentFinished {
            subagent_id: terminal.subagent_id,
            attempt_id: Some(terminal.result.attempt_id),
            child_session_id: terminal.child_session_id,
            status: status.to_owned(),
            error,
            tool_calls: terminal.result.tool_calls,
            turns: terminal.result.turns,
            duration_ms: terminal.result.duration_ms,
            tokens_used: terminal.result.tokens_used,
            output: Some(terminal.result.output.to_string()),
            will_wake: false,
        },
        meta: None,
    };
    let Ok(params) = serde_json::value::to_raw_value(&notification) else {
        return false;
    };
    session_notification::handle_session_notification_with_origin(
        &acp::ExtNotification::new("x.ai/session/update", params.into()),
        app,
        subagent_lifecycle::LifecycleOrigin::Reconciliation,
    );
    app.agents.values().any(|agent| {
        agent.session.session_id.as_ref() == Some(session_id)
            && agent.subagent_sessions.get(&child_session_id).is_some_and(|info| {
                info.attempt.lifecycle.current_attempt_id() == attempt_id && info.is_finished()
            })
    })
}
