use super::MvpAgent;
use crate::extensions::context_policy::{
    ContextPolicyApplyFailure, ContextPolicyFailure,
    ContextPolicyNotification, ContextPolicyReport, ContextPolicyResponse,
};
use crate::session::SessionHandle;
use xai_grok_sampling_types::{ContextPolicyEdit, ContextPolicyMutation, ContextScope};

impl MvpAgent {
    pub(crate) async fn edit_context_policy(
        &self,
        id: &agent_client_protocol::SessionId,
        origin: &SessionHandle,
        edit: &ContextPolicyEdit,
    ) -> Result<(), ContextPolicyFailure> {
        let store = self.context_policy_authority.clone().write_owned().await;
        if !self.resident_handle(id).is_some_and(|current| {
            current.cmd_tx.same_channel(&origin.cmd_tx)
                && current.context_policy_binding.incarnation == edit.expected.incarnation
        }) {
            return Err(ContextPolicyFailure::OriginChanged);
        }
        let session_id = id.0.to_string();
        let incarnation = origin.context_policy_binding.incarnation.clone();
        let edit_to_save = edit.clone();
        let (store, saved) = tokio::task::spawn_blocking(move || {
            let mut store = store;
            let result = store.edit(&session_id, &incarnation, &edit_to_save);
            (store, result)
        }).await.map_err(|_| ContextPolicyFailure::StorageWorkerLost)?;
        let durability = match saved {
            Ok(()) => crate::extensions::context_policy::ContextDurability::Confirmed,
            Err(crate::session::helpers::context_policy_store::ContextPolicyError::Durability(error)) => {
                crate::extensions::context_policy::ContextDurability::Unconfirmed { kind: error.kind().into(), os_code: error.raw_os_error() }
            }
            Err(error) => return Err(ContextPolicyFailure::from(&error)),
        };
        let global = match &edit.mutation {
            ContextPolicyMutation::SetCategories { target, .. } => *target == ContextScope::Global,
            ContextPolicyMutation::SelectScope { .. } | ContextPolicyMutation::ReplaceSession { .. } => false,
        };
        let mut handles = Vec::new();
        self.for_each_resident(|_, handle| handles.push(handle.clone()));
        for (_, child) in self.session_registry.child_hosts() {
            if let Some(handle) = child.running_handle() {
                handles.push(handle.clone());
            }
        }
        let mut failures = Vec::new();
        let mut notifications = Vec::new();
        for handle in handles {
            let sid = handle.info.id.0.as_ref();
            let result = if global || sid == id.0.as_ref() {
                let policy = store.effective(sid).map_err(|error| ContextPolicyFailure::from(&error))?;
                handle.chat_state_handle.set_context_policy(policy).await
            } else {
                handle.chat_state_handle.get_context_policy().await
            };
            let selection = store.selection(sid, &handle.context_policy_binding.incarnation)
                .map_err(|error| ContextPolicyFailure::from(&error))?;
            let response = match result {
                Ok(policy) => {
                    if policy == store.effective(sid).map_err(|error| ContextPolicyFailure::from(&error))? {
                        handle.context_policy_binding.applied_revision.store(
                            selection.version.revision, std::sync::atomic::Ordering::Release,
                        );
                    }
                    ContextPolicyResponse::Confirmed {
                        report: ContextPolicyReport { policy, usage: Vec::new(), selection: Some(selection) },
                    }
                },
                Err(error) => {
                    if global || sid == id.0.as_ref() {
                        failures.push(ContextPolicyApplyFailure {
                            session_id: sid.to_owned(), cause: (&error).into(),
                        });
                    }
                    // A lost reply is not evidence of non-application.
                    let current = handle.chat_state_handle.get_context_policy().await.ok()
                        .map(|policy| ContextPolicyReport { policy, usage: Vec::new(), selection: Some(selection) });
                    let failure = match crate::extensions::context_policy::ContextAdmissionFailure::from(&error) {
                        crate::extensions::context_policy::ContextAdmissionFailure::MailboxClosed => ContextPolicyFailure::MailboxClosed,
                        crate::extensions::context_policy::ContextAdmissionFailure::ReplyLost => ContextPolicyFailure::ReplyLost,
                    };
                    ContextPolicyResponse::Rejected { failure, current }
                }
            };
            notifications.push(ContextPolicyNotification { session_id: sid.to_owned(), response });
        }
        let failure = if !failures.is_empty() {
            Some(ContextPolicyFailure::Apply { failures, durability })
        } else {
            match durability {
                crate::extensions::context_policy::ContextDurability::Confirmed => None,
                crate::extensions::context_policy::ContextDurability::Unconfirmed { kind, os_code } => Some(ContextPolicyFailure::DurabilityUnconfirmed { kind, os_code }),
            }
        };
        drop(store);
        for mut notification in notifications {
            if notification.session_id == id.0.as_ref()
                && let Some(failure) = &failure
            {
                let current = match notification.response {
                    ContextPolicyResponse::Confirmed { report } => Some(report),
                    ContextPolicyResponse::Rejected { current, .. } => current,
                };
                notification.response = ContextPolicyResponse::Rejected { failure: failure.clone(), current };
            }
            crate::extensions::context_policy::publish_context_policy(&self.gateway, &notification);
        }
        match failure {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }
}
