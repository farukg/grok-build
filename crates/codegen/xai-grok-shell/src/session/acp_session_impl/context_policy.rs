use super::*;

impl SessionActor {
    /// Later requests are built under `policy`; the choice outlives the session actor.
    pub(super) fn set_context_policy(&self, policy: xai_grok_sampling_types::ContextPolicy) {
        crate::session::helpers::context_policy_store::save(
            &crate::session::persistence::session_dir(&self.session_info),
            &policy,
        );
        self.chat_state_handle.set_context_policy(policy);
    }
}
