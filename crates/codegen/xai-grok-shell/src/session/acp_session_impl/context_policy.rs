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

    pub(super) async fn context_policy_report(
        &self,
    ) -> crate::extensions::context_policy::ContextPolicyReport {
        let (policy, usage) = tokio::join!(
            self.chat_state_handle.get_context_policy(),
            self.chat_state_handle.get_context_usage(),
        );
        crate::extensions::context_policy::ContextPolicyReport { policy, usage }
    }
}
