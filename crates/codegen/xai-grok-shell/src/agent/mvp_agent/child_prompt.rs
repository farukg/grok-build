//! Human prompts addressed to a child session id.

use agent_client_protocol as acp;
use tokio::sync::{mpsc, oneshot};
use xai_grok_tools::implementations::grok_build::task::types::{
    ActiveAgentMessageOperation, ActiveAgentMessageOutcome, AgentAddress,
};

use super::{ChildHost, ChildReach, ChildResidence, MvpAgent, RunningChild, SessionHost};
use crate::agent::subagent::PromptTurnReceipt;
use crate::extensions::notification::SubagentDelivery;
use crate::extensions::subagent_deliver::DeliverSubagentOutcome;
use crate::extensions::subagent_resume::ResumeSubagentOutcome;
use crate::extensions::subagent_message::{SendSubagentMessageOutcome, literal_text};
use crate::session::SessionHandle;
use crate::session::commands::PromptTurnResult;

pub(super) enum ChildPromptAdmission {
    /// The prompt takes the root prompt path; its result settles a receipt so the parent still
    /// receives the child's last turn.
    Live {
        handle: Box<SessionHandle>,
        turn: ChildTurn,
    },
    /// The coordinator wakes the finished child under the same session id.
    Woken {
        parent_session_id: acp::SessionId,
        address: AgentAddress,
    },
    Refused(ChildPromptRefusal),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ChildPromptRefusal {
    #[error("workflow-owned child sessions do not accept prompts")]
    WorkflowOwned,
    #[error("the child session has too many prompts in flight")]
    Saturated,
}

impl ChildPromptAdmission {
    pub(super) fn of(child: ChildHost) -> Self {
        let ChildHost {
            parent_session_id,
            reach,
        } = child;
        match reach {
            ChildReach::Unaddressed => Self::Refused(ChildPromptRefusal::WorkflowOwned),
            ChildReach::Addressed {
                address,
                residence: ChildResidence::Finished,
            } => Self::Woken {
                parent_session_id,
                address,
            },
            ChildReach::Addressed {
                address,
                residence: ChildResidence::Running(running),
            } => {
                let RunningChild {
                    handle,
                    turns,
                    parent_prompt_index,
                    delivery,
                } = *running;
                match turns.try_reserve_owned() {
                    Ok(receipt) => {
                        // Held only after the reservation: a child whose drain already closed must
                        // not wait for a `deliver` nobody can send.
                        delivery.send_replace(SubagentDelivery::Held);
                        Self::Live {
                            handle: Box::new(handle),
                            turn: ChildTurn {
                                receipt,
                                parent_session_id,
                                parent_prompt_index,
                            },
                        }
                    }
                    // The receipt drain closes when the child's run finalizes; from then on only a
                    // wake reaches it.
                    Err(mpsc::error::TrySendError::Closed(_)) => Self::Woken {
                        parent_session_id,
                        address,
                    },
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        Self::Refused(ChildPromptRefusal::Saturated)
                    }
                }
            }
        }
    }
}

/// A reserved slot in the child's receipt drain.
pub(super) struct ChildTurn {
    receipt: mpsc::OwnedPermit<PromptTurnReceipt>,
    parent_session_id: acp::SessionId,
    parent_prompt_index: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl ChildTurn {
    /// Registers the dispatched prompt with the drain; the returned sender settles it.
    pub(super) fn admit(
        self,
        prompt_id: String,
        operation: ActiveAgentMessageOperation,
    ) -> oneshot::Sender<PromptTurnResult> {
        let (completion, result) = oneshot::channel();
        self.receipt.send(PromptTurnReceipt {
            prompt_id,
            result,
            telemetry: crate::session::telemetry::ActiveAgentMessageAdmissionTelemetry::new(
                std::time::Instant::now(),
                crate::agent::subagent::parent_telemetry_ctx(
                    &self.parent_session_id.0,
                    &self.parent_prompt_index,
                ),
                operation,
                operation,
                None,
            ),
        });
        completion
    }
}

impl MvpAgent {
    pub(crate) fn deliver_child(&self, id: &acp::SessionId) -> DeliverSubagentOutcome {
        let Some(SessionHost::Child(child)) = self.session_host(id) else {
            return DeliverSubagentOutcome::NotRunning;
        };
        match child.reach {
            ChildReach::Addressed {
                residence: ChildResidence::Running(running),
                ..
            } => {
                let released = running
                    .delivery
                    .send_if_modified(|delivery| match delivery {
                        SubagentDelivery::Held => {
                            *delivery = SubagentDelivery::OnTurnEnd;
                            true
                        }
                        SubagentDelivery::OnTurnEnd => false,
                    });
                if released {
                    DeliverSubagentOutcome::Delivered
                } else {
                    DeliverSubagentOutcome::NotHeld
                }
            }
            ChildReach::Addressed {
                residence: ChildResidence::Finished,
                ..
            }
            | ChildReach::Unaddressed => DeliverSubagentOutcome::NotRunning,
        }
    }

    pub(super) async fn wake_child_with_prompt(
        &self,
        parent_session_id: &acp::SessionId,
        child: &acp::SessionId,
        address: AgentAddress,
        prompt: Vec<acp::ContentBlock>,
    ) -> Result<acp::PromptResponse, acp::Error> {
        let text = prompt_text(prompt)?;
        match self
            .send_human_subagent_message(
                &parent_session_id.0,
                address.as_str().to_owned(),
                text.clone(),
                ActiveAgentMessageOperation::Queue,
            )
            .await
        {
            ActiveAgentMessageOutcome::Unsupported => {
                self.continue_child_with_prompt(parent_session_id, child, text).await
            }
            outcome @ ActiveAgentMessageOutcome::Accepted { .. } => {
                let wake =
                    serde_json::json!({ "childWake": SendSubagentMessageOutcome::from(outcome) });
                Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)
                    .meta(wake.as_object().cloned()))
            }
            // `ActiveAgentMessageOutcome` is `#[non_exhaustive]`; the wire outcome names every refusal.
            outcome => Err(refused_wake(SendSubagentMessageOutcome::from(outcome))),
        }
    }
}

impl MvpAgent {
    /// Without active agent messages a finished child continues the way `x.ai/subagent/resume`
    /// does: as a new child that starts from the finished one's history.
    pub(super) async fn continue_child_with_prompt(
        &self,
        parent_session_id: &acp::SessionId,
        child: &acp::SessionId,
        text: String,
    ) -> Result<acp::PromptResponse, acp::Error> {
        let outcome = ResumeSubagentOutcome::from(
            self.resume_subagent(&parent_session_id.0, &child.0, text).await,
        );
        let body = serde_json::json!({ "childResume": outcome });
        match outcome {
            ResumeSubagentOutcome::Queued { .. } | ResumeSubagentOutcome::Resumed { .. } => {
                Ok(acp::PromptResponse::new(acp::StopReason::EndTurn).meta(body.as_object().cloned()))
            }
            ResumeSubagentOutcome::Refused { .. } => {
                Err(acp::Error::invalid_request().data(body))
            }
        }
    }
}

pub(super) fn prompt_text(prompt: Vec<acp::ContentBlock>) -> Result<String, acp::Error> {
    literal_text(prompt).map_err(refused_wake)
}

/// The parent session a client names on a prompt to a child id this shell no longer knows
/// (`_meta.childOf`, stamped by clients that show child sessions).
pub(super) fn child_of(meta: Option<&acp::Meta>) -> Option<acp::SessionId> {
    meta?
        .get("childOf")?
        .as_str()
        .map(|parent| acp::SessionId::new(parent.to_owned()))
}

fn refused_wake(outcome: SendSubagentMessageOutcome) -> acp::Error {
    acp::Error::invalid_request().data(serde_json::json!({ "childWake": outcome }))
}
