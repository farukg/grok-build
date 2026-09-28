//! Human prompts addressed to a child session id.

use agent_client_protocol as acp;
use tokio::sync::{mpsc, oneshot};
use xai_grok_tools::implementations::grok_build::task::types::{
    ActiveAgentMessageOperation, ActiveAgentMessageOutcome, AgentAddress,
};

use super::{ChildHost, ChildReach, ChildResidence, MvpAgent, RunningChild};
use crate::agent::subagent::PromptTurnReceipt;
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
                } = *running;
                match turns.try_reserve_owned() {
                    Ok(receipt) => Self::Live {
                        handle: Box::new(handle),
                        turn: ChildTurn {
                            receipt,
                            parent_session_id,
                            parent_prompt_index,
                        },
                    },
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
    pub(super) async fn wake_child_with_prompt(
        &self,
        parent_session_id: &acp::SessionId,
        address: AgentAddress,
        prompt: Vec<acp::ContentBlock>,
    ) -> Result<acp::PromptResponse, acp::Error> {
        let text = literal_text(prompt).map_err(refused_wake)?;
        match self
            .send_human_subagent_message(
                &parent_session_id.0,
                address.as_str().to_owned(),
                text,
                ActiveAgentMessageOperation::Queue,
            )
            .await
        {
            ActiveAgentMessageOutcome::Unsupported => Err(acp::Error::invalid_request()
                .data("waking a finished child session requires features.active_agent_messages")),
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

fn refused_wake(outcome: SendSubagentMessageOutcome) -> acp::Error {
    acp::Error::invalid_request().data(serde_json::json!({ "childWake": outcome }))
}
