use super::*;
use crate::extensions::current_actions::PublicationContext;
use crate::session::{WakeCancelFailure, WakeCancelResponse, WakeCancelSettlement};

#[cfg(test)]
#[path = "cancel_settlement_tests.rs"]
mod tests;

impl SessionActor {
    fn accepts_wake_cancel(&self, context: &PublicationContext) -> bool {
        context.binding.session_id == self.session_info.id.0.as_ref()
            && !context.incarnation.is_nil()
            && self.current_actions.version().is_some_and(|version| {
                version.incarnation == context.incarnation
            })
    }

    pub(in crate::session::acp_session) async fn cancel_running_task_with_parent_teardown(
        &self,
        mut options: crate::session::CancelOptions,
        parent_request: ParentTeardownRequest,
    ) -> CancelOutcome {
        let (request, respond_to) = match options.target_prompt_id.take() {
            Some(crate::session::CancelTarget::Wake { request, respond_to }) => {
                (request, respond_to)
            }
            target @ (Some(crate::session::CancelTarget::Prompt(_)) | None) => {
                options.target_prompt_id = target;
                return self.cancel_running_task_inner(options, parent_request).await;
            }
        };
        let prompt_id = request.prompt_id.clone();
        options.target_prompt_id = Some(crate::session::CancelTarget::Prompt(prompt_id.clone()));
        let failure = if !self.accepts_wake_cancel(&request.context) {
            Some(WakeCancelFailure::SourceChanged)
        } else if !crate::session::PromptOrigin::from_prompt_id(&prompt_id).is_synthetic()
            || !matches!(options.history, crate::session::CancelHistoryDisposition::Keep)
        {
            Some(WakeCancelFailure::InvalidTarget)
        } else {
            None
        };
        let (outcome, settlement) = match failure {
            Some(reason) => (CancelOutcome::noop(), WakeCancelSettlement::Unresolved { reason }),
            None => {
                let outcome = self.cancel_running_task_inner(options, parent_request).await;
                let settlement = if self.current_actions.removed_wake(&prompt_id) {
                    WakeCancelSettlement::RemovedFromQueue
                } else {
                    self.read_wake_terminal(&prompt_id).await
                };
                (outcome, settlement)
            }
        };
        let settlement = if self.accepts_wake_cancel(&request.context) {
            settlement
        } else {
            WakeCancelSettlement::Unresolved { reason: WakeCancelFailure::SourceChanged }
        };
        let _ = respond_to.send(WakeCancelResponse {
            context: request.context,
            prompt_id,
            settlement,
        });
        outcome
    }

    async fn read_wake_terminal(&self, prompt_id: &str) -> WakeCancelSettlement {
        let (respond_to, response) = tokio::sync::oneshot::channel();
        if self.notifications.persistence_tx.send(PersistenceMsg::ReadTurnTerminal {
            prompt_id: prompt_id.to_owned(),
            respond_to,
        }).is_err() {
            return WakeCancelSettlement::Unresolved {
                reason: WakeCancelFailure::PersistenceUnavailable,
            };
        }
        match response.await {
            Ok(Ok(Some(notification))) => WakeCancelSettlement::Terminal { notification },
            Ok(Ok(None)) => WakeCancelSettlement::Unresolved {
                reason: WakeCancelFailure::UnknownTarget,
            },
            Ok(Err(_)) | Err(_) => WakeCancelSettlement::Unresolved {
                reason: WakeCancelFailure::PersistenceUnavailable,
            },
        }
    }
}
