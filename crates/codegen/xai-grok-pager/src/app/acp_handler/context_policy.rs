use super::AppView;
use agent_client_protocol as acp;
use xai_grok_shell::extensions::context_policy::ContextPolicyNotification;

pub(super) fn handle_context_policy_changed(notif: &acp::ExtNotification, app: &mut AppView) -> bool {
    let Ok(notification) = serde_json::from_str::<ContextPolicyNotification>(notif.params.get()) else {
        tracing::warn!("invalid Context policy notification");
        return false;
    };
    let mut changed = false;
    for (_, agent) in app.agents.all_mut() {
        if agent.session.session_id.as_ref().is_some_and(|id| id.0.as_ref() == notification.session_id) {
            agent.context_policy_notified(notification.response.clone());
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
#[path = "context_policy_tests.rs"]
mod tests;
