mod previous_completion;
use super::types::{
    ActiveAgentMessageOperation, ActiveAgentMessageSource, ActiveMessageReceiver,
    ActiveMessageRoute, SubagentRequest, SubagentRuntimeOverrides,
};
pub use previous_completion::PreviousCompletion;
use serde::{Deserialize, Serialize};

pub fn payload_digest(payload: &str) -> String {
    xai_file_utils::sha256_hex(payload.as_bytes())
}

pub fn parent_message_prompt_id(message_id: &str) -> String {
    format!("parent-message-{message_id}")
}
pub fn parent_message_delivery_id(message_id: &str) -> String {
    format!("parent-message/{message_id}")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecoveryRef {
    pub delivery_id: String,
    pub message_id: String,
    pub prompt_id: String,
    pub receiver: ActiveMessageReceiver,
    pub operation: ActiveAgentMessageOperation,
    pub payload_digest: String,
}

impl std::fmt::Display for MessageRecoveryRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match serde_json::to_string(self) {
            Ok(reference) => f.write_str(&reference),
            Err(_) => Err(std::fmt::Error),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactCapabilityRef {
    Disabled,
    Direct {
        bucket_url: String,
        principal: ArtifactDirectPrincipal,
    },
    Proxy {
        destination: ArtifactProxyDestination,
        endpoint_binding: String,
        principal: ArtifactProxyPrincipal,
    },
    S3 {
        bucket_url: String,
        bucket: String,
        region: String,
        endpoint_binding: Option<String>,
        principal: ArtifactS3Principal,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactDirectPrincipal {
    Unavailable,
    ServiceAccount {
        client_email: String,
        project_id: Option<String>,
        token_audience_binding: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactS3Principal {
    Unavailable,
    CredentialFile { binding: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactProxyDestination {
    Managed,
    Bucket { url: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactProxyPrincipal {
    Unavailable,
    Login {
        user_id: String,
        principal_id: Option<String>,
        team_id: Option<String>,
        organization_id: Option<String>,
        scope: ArtifactProxyScope,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactProxyScope {
    Standard,
    Staging,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeWakeLaunch {
    pub parent_session_id: String,
    pub description: String,
    pub subagent_type: String,
    pub cwd: String,
    pub runtime_overrides: SubagentRuntimeOverrides,
    pub surface_completion: bool,
    pub tool_call_id: Option<String>,
    pub attempt_id: String,
    pub previous_completion: PreviousCompletion,
    pub publication_started_at: chrono::DateTime<chrono::Utc>,
    pub turn_number: u64,
    pub agent_address: String,
    pub spawner_session_id: Option<String>,
    pub traceparent: Option<String>,
    pub parent_prompt_index: usize,
    pub publication_agent_name: String,
    pub publication_role_name: Option<String>,
    pub publication_background: bool,
    pub publication_reasoning_effort: Option<xai_grok_sampling_types::ReasoningEffort>,
    pub artifacts: ArtifactCapabilityRef,
    pub management_recipe_binding: String,
    pub inherited_mcp_servers: Vec<String>,
    pub owned_mcp_servers: Vec<String>,
    pub tool_overrides: Option<xai_grok_sampling_types::ToolOverrides>,
}

impl NativeWakeLaunch {
    pub fn request(&self, reference: &MessageRecoveryRef, text: String) -> SubagentRequest {
        SubagentRequest {
            id: reference.receiver.session_id.clone(),
            prompt: text,
            description: self.description.clone(),
            subagent_type: self.subagent_type.clone(),
            parent_session_id: self.parent_session_id.clone(),
            parent_prompt_id: None,
            resume_from: None,
            cwd: Some(self.cwd.clone()),
            runtime_overrides: self.runtime_overrides.clone(),
            run_in_background: true,
            surface_completion: self.surface_completion,
            await_to_completion: false,
            fork_context: false,
            owner: super::types::SubagentOwner::Task,
            cancel_token: tokio_util::sync::CancellationToken::new(),
            spawn_root: super::types::SpawnRootSpan::from_traceparent(self.traceparent.clone()),
            tool_call_id: self.tool_call_id.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeWakeCustody {
    pub receiver: ActiveMessageReceiver,
    pub operation: ActiveAgentMessageOperation,
    pub route: ActiveMessageRoute,
    pub launch: NativeWakeLaunch,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRecoveryAction {
    Observe,
    Adopt,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRecoveryState {
    Queued,
    Processing,
    Interrupted,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub enum PreparedMessageRecovery {
    Observed {
        reference: MessageRecoveryRef,
        state: MessageRecoveryState,
    },
    Initial {
        custody: Box<NativeWakeCustody>,
        message: super::types::ActiveAgentMessage,
        source: ActiveAgentMessageSource,
        publication: RecoveryPublication,
    },
    Unavailable {
        reference: MessageRecoveryRef,
    },
    Refused,
}

#[derive(Clone, Debug)]
pub struct MessageRecoveryAudience {
    parent_session_id: String,
    actor: RecoveryActor,
}

#[derive(Clone, Debug)]
enum RecoveryActor {
    Parent,
    Sender { session_id: String },
}

impl MessageRecoveryAudience {
    pub(crate) fn parent(parent: &str) -> Self {
        Self {
            parent_session_id: parent.to_owned(),
            actor: RecoveryActor::Parent,
        }
    }
    pub(crate) fn sender(parent: &str, sender: &str) -> Self {
        Self {
            parent_session_id: parent.to_owned(),
            actor: RecoveryActor::Sender {
                session_id: sender.to_owned(),
            },
        }
    }
    pub fn parent_session_id(&self) -> &str {
        &self.parent_session_id
    }
    pub fn permits_sender(&self, original_sender: &str, source: ActiveAgentMessageSource) -> bool {
        match &self.actor {
            RecoveryActor::Parent => true,
            RecoveryActor::Sender { session_id } => {
                session_id == original_sender && source == ActiveAgentMessageSource::Agent
            }
        }
    }
}

#[derive(Debug)]
pub struct SubagentMessageRecoveryRequest {
    pub(crate) permit: Option<tokio::sync::OwnedSemaphorePermit>,
    pub reference: MessageRecoveryRef,
    pub action: MessageRecoveryAction,
    pub sender_context: super::types::ActiveMessageSenderContext,
    pub respond_to: tokio::sync::oneshot::Sender<super::types::ActiveAgentMessageOutcome>,
}

pub(crate) async fn dispatch_recovery(
    tx: &tokio::sync::mpsc::UnboundedSender<super::types::SubagentEvent>,
    permit: tokio::sync::OwnedSemaphorePermit,
    reference: MessageRecoveryRef,
    action: MessageRecoveryAction,
    sender_context: super::types::ActiveMessageSenderContext,
) -> super::types::ActiveAgentMessageOutcome {
    use super::types::{ActiveAgentMessageOutcome, SubagentEvent};
    let (respond_to, response) = tokio::sync::oneshot::channel();
    let command = SubagentMessageRecoveryRequest {
        permit: Some(permit),
        reference: reference.clone(),
        action,
        sender_context,
        respond_to,
    };
    if tx.send(SubagentEvent::RecoverMessage(command)).is_err() {
        return ActiveAgentMessageOutcome::AdmissionUncertain {
            recovery_ref: reference,
        };
    }
    response
        .await
        .unwrap_or(ActiveAgentMessageOutcome::AdmissionUncertain {
            recovery_ref: reference,
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialPublication {
    Unpublished,
    Published,
}

#[derive(Clone, Debug)]
pub enum RecoveryPublication {
    Unpublished { previous_prompt: String },
    Published,
}

impl RecoveryPublication {
    pub fn kind(&self) -> InitialPublication {
        match self {
            Self::Unpublished { .. } => InitialPublication::Unpublished,
            Self::Published => InitialPublication::Published,
        }
    }
}

#[derive(Clone, Debug)]
pub enum NativeWakeMode {
    Admit,
    Adopt {
        custody: Box<NativeWakeCustody>,
        attempt_id: xai_message_delivery_core::AttemptId,
        publication: RecoveryPublication,
    },
}

#[cfg(test)]
pub(crate) fn wire_reference_fixture()
-> crate::implementations::grok_build::task::native_recovery::MessageRecoveryRef {
    crate::implementations::grok_build::task::native_recovery::MessageRecoveryRef {
        delivery_id: "parent-message/message".into(),
        message_id: "message".into(),
        prompt_id: "parent-message-message".into(),
        receiver: crate::implementations::grok_build::task::types::ActiveMessageReceiver {
            session_id: "receiver".into(),
            generation:
                crate::implementations::grok_build::task::root_control::AgentMessageGeneration::mint(
                    42,
                ),
        },
        operation: ActiveAgentMessageOperation::Queue,
        payload_digest: "00".repeat(32),
    }
}
