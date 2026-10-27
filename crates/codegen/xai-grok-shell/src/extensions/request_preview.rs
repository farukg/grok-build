//! Read-only next-request candidates, separate from actual request evidence.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};
use xai_grok_sampling_types::{ContextPolicy, ProjectedRequestMember};
use xai_grok_sampler::request_preview::NativePreview;
use super::{ExtResult, to_raw_response};
use crate::agent::MvpAgent;

pub const REQUEST_PREVIEW_METHOD: &str = "x.ai/session/request_preview";

#[derive(Clone)]
pub struct RequestPreviewRequest {
    pub session_id: String,
    pub ui_generation: u64,
    pub input: PreviewInput,
}

#[derive(Clone)]
pub enum PreviewInput {
    Current,
    Draft {
        delivery: PreviewDelivery,
        blocks: Vec<acp::ContentBlock>,
        json_schema: Option<serde_json::Value>,
    },
}

impl std::fmt::Debug for RequestPreviewRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestPreviewRequest").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewDelivery { Prompt, Queue, Interjection, SendNow }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewKey {
    pub session_id: String,
    pub binding: String,
    pub canonical_history_revision: [u8; 32],
    pub context_policy_revision: u64,
    pub ui_generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewInputPosition {
    Current,
    Draft { delivery: PreviewDelivery },
    BehindQueuedInput,
    QueuedInput,
    DeferredInterjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewLateBound {
    ActiveWork,
    UpcomingToolResults,
    ActiveStructuredOutput,
    ScheduledInjections,
    PendingInterjections,
    QueuePromotion,
    AttachmentPreparation,
    FileExpansion,
    SkillOrCommandExpansion,
    LargePromptOffload,
    InterruptEnvelope,
    FirstTurnMemory,
    PlanReminders,
    PromptModePreparation,
    ModelConfigurationRefresh,
    ToolOverridePromotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewUnavailable {
    SessionNotResident,
    BindingChanged,
    SessionMailboxClosed,
    SessionReplyLost,
    ChatMailboxClosed,
    ChatReplyLost,
    SamplerMailboxClosed,
    SamplerReplyLost,
    AuthorityNotLoaded,
    LegacyPolicyNotInspected,
    ModelNotInitialized,
    ModelConfigurationChanged,
    SchemaNotPrepared,
    Serialization,
    OutputBudget,
}

pub struct PreviewSnapshot {
    pub key: PreviewKey,
    pub context_policy: ContextPolicy,
    pub input_position: PreviewInputPosition,
    pub membership: Vec<ProjectedRequestMember>,
    pub native: NativePreview,
    pub projected_items: Vec<xai_grok_sampling_types::ConversationItem>,
    pub working_directory: std::path::PathBuf,
}

impl std::fmt::Debug for PreviewSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewSnapshot").field("key", &self.key).finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum RequestPreview {
    CompleteAsOf { snapshot: PreviewSnapshot },
    Partial { snapshot: PreviewSnapshot, first: PreviewLateBound, additional: Vec<PreviewLateBound> },
    Unavailable { session_id: String, ui_generation: u64, cause: PreviewUnavailable },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PublishedRequestPreview {
    CompleteAsOf { snapshot: PublishedPreviewSnapshot },
    Partial { snapshot: PublishedPreviewSnapshot, first: PreviewLateBound, additional: Vec<PreviewLateBound> },
    Unavailable { session_id: String, ui_generation: u64, cause: PreviewUnavailable },
}

#[derive(Serialize, Deserialize)]
pub struct PublishedPreviewSnapshot {
    pub key: PreviewKey,
    pub context_policy: ContextPolicy,
    pub input_position: PreviewInputPosition,
    pub membership: Vec<ProjectedRequestMember>,
    pub backend: xai_grok_sampling_types::ApiBackend,
    pub model: String,
    pub representation: xai_grok_sampler::request_capture::PayloadRepresentation,
    pub candidate_body: Box<serde_json::value::RawValue>,
}

impl std::fmt::Debug for PublishedPreviewSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublishedPreviewSnapshot").field("key", &self.key).finish_non_exhaustive()
    }
}

impl PreviewSnapshot {
    fn publication(self) -> Result<PublishedPreviewSnapshot, serde_json::Error> {
        let body = self.native.body.publication_view()?;
        Ok(PublishedPreviewSnapshot {
            key: self.key, context_policy: self.context_policy, input_position: self.input_position,
            membership: self.membership, backend: self.native.backend, model: self.native.model,
            representation: body.representation,
            candidate_body: serde_json::from_slice(&body.bytes)?,
        })
    }
}

impl RequestPreview {
    pub fn unavailable(request: &RequestPreviewRequest, cause: PreviewUnavailable) -> Self {
        Self::Unavailable { session_id: request.session_id.clone(), ui_generation: request.ui_generation, cause }
    }

    pub fn publication(self) -> Result<PublishedRequestPreview, serde_json::Error> {
        match self {
            Self::CompleteAsOf { snapshot } => Ok(PublishedRequestPreview::CompleteAsOf { snapshot: snapshot.publication()? }),
            Self::Partial { snapshot, first, additional } => Ok(PublishedRequestPreview::Partial { snapshot: snapshot.publication()?, first, additional }),
            Self::Unavailable { session_id, ui_generation, cause } => Ok(PublishedRequestPreview::Unavailable { session_id, ui_generation, cause }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestPreviewRead {
    pub session_id: String,
    pub ui_generation: u64,
}

pub(crate) async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    // ACP 0.10.4 traces complete RPC lines; drafts use only the local typed command.
    let read: RequestPreviewRead = serde_json::from_str(args.params.get())
        .map_err(|_| acp::Error::invalid_params().data("invalid request-preview parameters"))?;
    let request = RequestPreviewRequest {
        session_id: read.session_id, ui_generation: read.ui_generation, input: PreviewInput::Current,
    };
    let id = acp::SessionId::new(request.session_id.clone());
    let result = if let Some(handle) = agent.resident_handle(&id) {
        let (respond_to, response) = tokio::sync::oneshot::channel();
        if handle.cmd_tx.send(crate::session::SessionCommand::PreviewRequest {
            request: request.clone(), respond_to,
        }).is_err() {
            RequestPreview::unavailable(&request, PreviewUnavailable::SessionMailboxClosed)
        } else {
            let result = match response.await {
                Ok(result) => result,
                Err(_) => RequestPreview::unavailable(&request, PreviewUnavailable::SessionReplyLost),
            };
            if agent.resident_handle(&id).is_some_and(|current|
                current.cmd_tx.same_channel(&handle.cmd_tx)
                && current.context_policy_binding.incarnation == handle.context_policy_binding.incarnation)
            {
                result
            } else {
                RequestPreview::unavailable(&request, PreviewUnavailable::BindingChanged)
            }
        }
    } else {
        RequestPreview::unavailable(&request, PreviewUnavailable::SessionNotResident)
    };
    let published = result.publication()
        .map_err(|_| acp::Error::internal_error().data("request-preview publication failed"))?;
    to_raw_response(&published)
}
