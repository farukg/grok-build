use super::*;

#[cfg(test)]
#[path = "context_policy_tests.rs"]
mod tests;

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
        let (policy, mut usage, definitions) = tokio::join!(
            self.chat_state_handle.get_context_policy(),
            self.chat_state_handle.get_context_usage(),
            self.prepare_tool_definitions_inner(),
        );
        let tools = self.effective_function_tools(&definitions).await;
        usage.push(xai_grok_sampling_types::CategoryTokens {
            category: xai_grok_sampling_types::ContextCategory::ToolDefinitions,
            tokens: xai_chat_state::estimate_tool_specs_tokens(&tools),
        });
        crate::extensions::context_policy::ContextPolicyReport { policy, usage }
    }
}
