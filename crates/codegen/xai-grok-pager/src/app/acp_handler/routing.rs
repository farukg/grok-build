use super::*;

/// Result of looking up which session view a notification targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SessionMatch(pub(super) AgentId);

impl SessionMatch {
    pub(super) fn agent_id(self) -> AgentId {
        self.0
    }
}

/// The session and scrollback that own the task rows for `session_id`.
pub(crate) fn task_view_by_session_id<'a>(
    app: &'a mut AppView,
    session_id: &str,
) -> Option<(
    &'a mut AgentSession,
    &'a mut crate::scrollback::state::ScrollbackState,
)> {
    let (_, _, agent) = resolve_notif_agent(app, &acp::SessionId::new(session_id))?;
    Some((&mut agent.session, &mut agent.scrollback))
}

/// Resolve the agent that owns a notification's `session_id` and whether the active view is affected.
///
/// Convenience wrapper around `find_session_match`, `is_matched_agent_active`, and `agents.get_mut()`, used by the bg-task notification handlers.
pub(super) fn resolve_notif_agent<'a>(
    app: &'a mut AppView,
    session_id: &acp::SessionId,
) -> Option<(SessionMatch, bool, &'a mut AgentView)> {
    let matched = find_session_match(app, session_id)?;
    let agent_id = matched.agent_id();
    let is_active = is_matched_agent_active(app, agent_id);
    let agent = app.agents.get_mut(&agent_id)?;
    Some((matched, is_active, agent))
}

/// A session's progress updates and completion signal land on its matching agent.
/// Resolves by session id when provided; otherwise uses the active session.
pub(super) fn mcp_target_agent<'a>(
    app: &'a mut AppView,
    session_id: Option<&str>,
) -> Option<(bool, &'a mut AgentView)> {
    match session_id {
        Some(sid) => {
            let sid = acp::SessionId::new(sid);
            let (_, is_active, agent) = resolve_notif_agent(app, &sid)?;
            Some((is_active, agent))
        }
        None => {
            let id = match app.active_view {
                ActiveView::Agent(id) => id,
                ActiveView::Welcome => app.home_session_agent?,
                ActiveView::AgentDashboard => return None,
            };
            let agent = app.agents.get_mut(&id)?;
            Some((matches!(app.active_view, ActiveView::Agent(_)), agent))
        }
    }
}

/// The in-flight create a setup-phase notification targets, matched only by `pending_session_id`
/// (not the bound id, so a late phase can't re-stain a live session; no active-view fallback).
pub(super) fn setup_phase_target_agent<'a>(
    app: &'a mut AppView,
    session_id: &str,
) -> Option<&'a mut AgentView> {
    let sid = acp::SessionId::new(session_id);
    app.agents
        .values_mut()
        .find(|agent| agent.pending_session_id.as_ref() == Some(&sid))
}


/// The only agent that could own such a pre-assignment notification is the one the user just created (necessarily active, `session_id == None`).
/// Returns `None` when the notification cannot be associated with any agent.
/// All ACP-notification handlers must route through this function rather than gating on `app.active_view` directly.
pub(super) fn find_session_match(
    app: &AppView,
    session_id: &acp::SessionId,
) -> Option<SessionMatch> {
    for (id, agent) in &app.agents {
        if agent.session.session_id.as_ref() == Some(session_id) {
            return Some(SessionMatch(*id));
        }
    }
    // Pass 3: race-window fallback for notifications that arrive before the root session_id has been assigned
    // Only the active agent is eligible, and only when its `session_id` is still `None`
    // Otherwise we would misroute a stranger's notification to whichever agent happens to be foregrounded
    if let ActiveView::Agent(active_id) = app.active_view
        && let Some(agent) = app.agents.get(&active_id)
        && agent.session.session_id.is_none()
    {
        return Some(SessionMatch(active_id));
    }
    if matches!(app.active_view, ActiveView::Welcome)
        && let Some(id) = app.home_session_agent
        && let Some(agent) = app.agents.get(&id)
        && agent.session.session_id.is_none()
    {
        return Some(SessionMatch(id));
    }
    None
}

/// Whether the matched agent is the one currently displayed.
pub(super) fn is_matched_agent_active(app: &AppView, matched_agent: AgentId) -> bool {
    matches!(app.active_view, ActiveView::Agent(id) if id == matched_agent)
}

/// Routes by the request's session id via [`find_session_match`] (exactly like `session/update` notifications), not gated on `app.active_view`.
/// A modal raised by a **background** session thus lands on its own view even when the user is on the dashboard or a different session.
/// The caller must then leave the reverse-request unanswered (drop, do NOT error) and rely on the leader's replay-on-attach.
pub(super) fn interaction_target_agent(app: &AppView, session_id: &str) -> Option<AgentId> {
    let sid = acp::SessionId::new(session_id.to_owned());
    find_session_match(app, &sid).map(SessionMatch::agent_id)
}
