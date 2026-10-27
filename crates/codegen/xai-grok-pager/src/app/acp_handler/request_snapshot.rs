use super::AppView;
use agent_client_protocol as acp;
use xai_grok_shell::session::request_capture::RequestSnapshotNotification;

/// `x.ai/session/request_snapshot/observed`: the shell captured a real request event.
/// Refreshes the inspector only for the session the observation belongs to; nothing else reacts.
pub(super) fn handle_request_snapshot_observed(notif: &acp::ExtNotification, app: &mut AppView) -> bool {
    let Ok(notification) = serde_json::from_str::<RequestSnapshotNotification>(notif.params.get()) else {
        tracing::warn!("invalid request snapshot notification");
        return false;
    };
    let mut changed = false;
    let mut refreshes = Vec::new();
    for (agent_id, agent) in app.agents.all_mut() {
        if agent
            .session
            .session_id
            .as_ref()
            .is_none_or(|id| id.0.as_ref() != notification.snapshot.session_id.as_str())
        {
            continue;
        }
        if let Some(fetch) = agent.request_snapshot_observed(&notification) {
            refreshes.push((agent_id, fetch));
        }
        changed = true;
    }
    for (agent_id, (session_id, query, generation)) in refreshes {
        app.pending_effects
            .push(crate::app::actions::Effect::RequestSnapshotQuery {
                agent_id,
                session_id,
                query,
                generation,
            });
    }
    changed
}

#[cfg(test)]
#[path = "request_snapshot_tests.rs"]
mod tests;
