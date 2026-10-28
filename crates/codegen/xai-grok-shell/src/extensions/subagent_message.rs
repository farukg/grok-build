//! `x.ai/subagent/message` — queue or steer literal text to an owned child.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};
use xai_grok_tools::implementations::grok_build::send_subagent_message::resolve_delivery;
use xai_grok_tools::implementations::grok_build::task::types::{
    ActiveAgentMessageOutcome, MAX_ACTIVE_AGENT_MESSAGE_BYTES,
};

use crate::agent::MvpAgent;
use crate::session::ExtMethodResult;

use super::ExtResult;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendSubagentMessageRequest {
    pub session_id: String,
    pub agent_address: String,
    #[serde(default)]
    pub queue: bool,
    pub content: Vec<acp::ContentBlock>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(
    rename_all = "snake_case",
    tag = "kind",
    rename_all_fields = "camelCase"
)]
pub enum SendSubagentMessageOutcome {
    Accepted {
        message_id: String,
    },
    Rejected,
    /// The address is owned but the child is not yet active. Callers should retry.
    NotActive,
    UnsupportedContent,
    Limit {
        max_bytes: usize,
        observed_bytes: usize,
    },
    AdmissionUncertain { recovery_ref: xai_grok_tools::implementations::grok_build::task::native_recovery::MessageRecoveryRef },
    RecoveryObserved { recovery_ref: xai_grok_tools::implementations::grok_build::task::native_recovery::MessageRecoveryRef, state: xai_grok_tools::implementations::grok_build::task::native_recovery::MessageRecoveryState },
    NotAcceptedBeforeDeadline,
    Saturated {
        max_in_flight: usize,
    },
    ChannelClosed,
}

pub(crate) fn literal_text(
    content: Vec<acp::ContentBlock>,
) -> Result<String, SendSubagentMessageOutcome> {
    if content.len() != 1 {
        return Err(SendSubagentMessageOutcome::UnsupportedContent);
    }
    let Some(acp::ContentBlock::Text(text)) = content.into_iter().next() else {
        return Err(SendSubagentMessageOutcome::UnsupportedContent);
    };
    if text.text.is_empty() || text.text.len() > MAX_ACTIVE_AGENT_MESSAGE_BYTES {
        return Err(SendSubagentMessageOutcome::Limit {
            max_bytes: MAX_ACTIVE_AGENT_MESSAGE_BYTES,
            observed_bytes: text.text.len(),
        });
    }
    Ok(text.text)
}

impl From<ActiveAgentMessageOutcome> for SendSubagentMessageOutcome {
    fn from(outcome: ActiveAgentMessageOutcome) -> Self {
        match outcome {
            ActiveAgentMessageOutcome::Accepted { message_id } => Self::Accepted { message_id },
            ActiveAgentMessageOutcome::NotActiveOrFinalizing => Self::NotActive,
            ActiveAgentMessageOutcome::NotFoundOrNotOwned
            | ActiveAgentMessageOutcome::Unsupported => Self::Rejected,
            ActiveAgentMessageOutcome::Saturated { max_in_flight } => {
                Self::Saturated { max_in_flight }
            }
            ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref } => Self::AdmissionUncertain { recovery_ref },
            ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref, state } => Self::RecoveryObserved { recovery_ref, state },
            ActiveAgentMessageOutcome::NotAcceptedBeforeDeadline => Self::NotAcceptedBeforeDeadline,
            ActiveAgentMessageOutcome::Limit {
                max_bytes,
                observed_bytes,
            } => Self::Limit {
                max_bytes,
                observed_bytes,
            },
            ActiveAgentMessageOutcome::ChannelClosed => Self::ChannelClosed,
            _ => Self::Rejected,
        }
    }
}

fn respond<T: Serialize>(result: Result<T, impl std::fmt::Display>) -> ExtResult {
    ExtMethodResult::from_result(result)
        .to_ext_response()
        .map_err(|e| acp::Error::internal_error().data(e.to_string()))
}

pub(crate) async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    if !agent
        .cfg
        .borrow()
        .is_feature_enabled(crate::agent::config::Feature::ActiveAgentMessages)
    {
        return Err(acp::Error::method_not_found());
    }
    let req: SendSubagentMessageRequest = super::parse_params(args)?;
    let text = match literal_text(req.content) {
        Ok(text) => text,
        Err(outcome) => return respond(Ok::<_, String>(outcome)),
    };
    let operation = resolve_delivery(/*delivery*/ None, req.queue);
    let outcome = agent
        .send_human_subagent_message(&req.session_id, req.agent_address, text, operation)
        .await;
    respond(Ok::<_, String>(SendSubagentMessageOutcome::from(outcome)))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecoverSubagentMessageRequest {
    pub(crate) session_id: String,
    pub(crate) recovery_ref: xai_grok_tools::implementations::grok_build::task::native_recovery::MessageRecoveryRef,
    pub(crate) action: xai_grok_tools::implementations::grok_build::task::native_recovery::MessageRecoveryAction,
}

pub(crate) async fn handle_recovery(
    agent: &MvpAgent,
    args: &acp::ExtRequest,
    coordinator: &xai_grok_tools::implementations::grok_build::task::backend::SubagentCoordinatorSender,
) -> ExtResult {
    if !agent.cfg.borrow().is_feature_enabled(crate::agent::config::Feature::ActiveAgentMessages) {
        return Err(acp::Error::method_not_found());
    }
    let request: RecoverSubagentMessageRequest = super::parse_params(args)?;
    if agent.session_handle_waiting_for_load(&acp::SessionId::new(request.session_id.clone())).await.is_none() {
        return respond(Ok::<_, String>(SendSubagentMessageOutcome::Rejected));
    }
    let outcome = coordinator.recover_message(request.recovery_ref, request.action,
        xai_grok_tools::implementations::grok_build::task::types::ActiveMessageSenderContext::HumanRoot {
            session_id: std::sync::Arc::from(request.session_id),
        }).await;
    respond(Ok::<_, String>(SendSubagentMessageOutcome::from(outcome)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_subagent_message_request_omitted_queue_is_steer() {
        let req: SendSubagentMessageRequest = serde_json::from_str(
            r#"{"sessionId":"parent","agentAddress":"opaque","content":[{"type":"text","text":"hi"}]}"#,
        )
        .expect("parse");
        assert!(!req.queue);
        assert_eq!(req.agent_address, "opaque");
    }

    #[test]
    fn literal_text_rejects_image_blocks() {
        let blocks: Vec<acp::ContentBlock> =
            serde_json::from_str(r#"[{"type":"image","data":"abc","mimeType":"image/png"}]"#)
                .expect("image block");
        assert_eq!(
            literal_text(blocks),
            Err(SendSubagentMessageOutcome::UnsupportedContent)
        );
    }

    #[test]
    fn admission_uncertainty_retains_exact_recovery_reference_in_extension_output() {
        use xai_grok_tools::implementations::grok_build::task::{native_recovery::MessageRecoveryRef, root_control::AgentMessageGeneration, types::{ActiveAgentMessageOperation, ActiveMessageReceiver}};
        let reference = MessageRecoveryRef {
            delivery_id: "parent-message/message".into(), message_id: "message".into(),
            prompt_id: "parent-message-message".into(), payload_digest: "ab".repeat(32),
            receiver: ActiveMessageReceiver { session_id: "receiver".into(), generation: AgentMessageGeneration::mint(42) },
            operation: ActiveAgentMessageOperation::Queue,
        };
        let output = SendSubagentMessageOutcome::from(ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref: reference.clone() });
        let wire = serde_json::to_value(output).unwrap();
        assert_eq!(wire["kind"], "admission_uncertain");
        assert_eq!(wire["recoveryRef"], serde_json::to_value(&reference).unwrap());
        let request: RecoverSubagentMessageRequest = serde_json::from_value(serde_json::json!({
            "sessionId": "authenticated-parent", "action": "observe", "recoveryRef": reference,
        })).unwrap();
        assert_eq!(request.action, xai_grok_tools::implementations::grok_build::task::native_recovery::MessageRecoveryAction::Observe);
    }

    #[test]
    fn not_active_outcome_is_retryable_kind() {
        let outcome =
            SendSubagentMessageOutcome::from(ActiveAgentMessageOutcome::NotActiveOrFinalizing);
        assert_eq!(outcome, SendSubagentMessageOutcome::NotActive);
        let json = serde_json::to_value(&outcome).expect("serialize");
        assert_eq!(json.get("kind"), Some(&serde_json::json!("not_active")));
    }
}
