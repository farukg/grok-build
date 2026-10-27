use super::*;
use crate::session::helpers::context_policy_store::ContextPolicyError;
#[cfg(test)]
use xai_grok_sampling_types::{ContextPolicy, ContextPolicyEdit, ContextPolicyMutation};

#[cfg(test)]
#[path = "context_policy_tests.rs"]
mod tests;

impl SessionActor {
    pub(crate) async fn admit_context_policy_for_request(
        &self,
    ) -> Result<tokio::sync::RwLockReadGuard<'_, crate::session::helpers::context_policy_store::ContextPolicyStore>, ContextPolicyError> {
        let store = self.context_policy_binding.authority.read().await;
        let id = self.session_info.id.0.as_ref();
        let selection = store.selection(id, &self.context_policy_binding.incarnation)?;
        if self.context_policy_binding.applied_revision.load(std::sync::atomic::Ordering::Acquire) != selection.version.revision {
            let policy = self.chat_state_handle.set_context_policy(store.effective(id)?).await?;
            self.context_policy_binding.applied_revision.store(selection.version.revision, std::sync::atomic::Ordering::Release);
            crate::extensions::context_policy::publish_context_policy(&self.notifications.gateway,
                &crate::extensions::context_policy::ContextPolicyNotification {
                    session_id: id.to_owned(),
                    response: crate::extensions::context_policy::ContextPolicyResponse::Confirmed {
                        report: crate::extensions::context_policy::ContextPolicyReport { policy, usage: Vec::new(), selection: Some(selection) },
                    },
                },
            );
        }
        Ok(store)
    }

    #[cfg(test)]
    pub(super) async fn set_context_policy(&self, policy: ContextPolicy) -> Result<ContextPolicy, ContextPolicyError> {
        let mut store = self.context_policy_binding.authority.write().await;
        let id = self.session_info.id.0.as_ref();
        store.ensure_session(id, &crate::session::persistence::session_dir(&self.session_info))?;
        let expected = store.selection(id, &self.context_policy_binding.incarnation)?.version;
        store.edit(id, &self.context_policy_binding.incarnation, &ContextPolicyEdit {
            expected, mutation: ContextPolicyMutation::ReplaceSession { policy },
        })?;
        Ok(self.chat_state_handle.set_context_policy(store.effective(id)?).await?)
    }

    pub(super) async fn context_policy_report(
        &self,
    ) -> Result<crate::extensions::context_policy::ContextPolicyReport, ContextPolicyError> {
        let snapshot = async {
            let store = self.admit_context_policy_for_request().await?;
            let selection = store.selection(self.session_info.id.0.as_ref(), &self.context_policy_binding.incarnation)?;
            let (policy, usage) = tokio::join!(
                self.chat_state_handle.get_context_policy(),
                self.chat_state_handle.get_context_usage(),
            );
            Ok::<_, ContextPolicyError>((policy?, usage, selection))
        };
        let (snapshot, definitions) = tokio::join!(snapshot, self.prepare_tool_definitions_inner());
        let (policy, mut usage, selection) = snapshot?;
        let tools = self.effective_function_tools(&definitions).await;
        usage.push(xai_grok_sampling_types::CategoryTokens {
            category: xai_grok_sampling_types::ContextCategory::ToolDefinitions,
            tokens: xai_chat_state::estimate_tool_specs_tokens(&tools),
        });
        Ok(crate::extensions::context_policy::ContextPolicyReport { policy, usage, selection: Some(selection) })
    }
}
