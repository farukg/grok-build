use crate::extensions::current_actions::PublicationContext;
use crate::extensions::notification::SessionNotification;

pub const CANCEL_SETTLEMENT_METHOD: &str = "x.ai/session/cancel_settlement";

pub(crate) fn find_turn_terminal(
    updates: Vec<super::super::storage::SessionUpdate>,
    session_id: &str,
    prompt_id: &str,
) -> Option<SessionNotification> {
    super::super::storage::filter_rewind_updates(updates)
        .into_iter()
        .rev()
        .find_map(|update| match update {
            super::super::storage::SessionUpdate::Xai(notification)
                if notification.session_id.0.as_ref() == session_id
                    && matches!(&notification.update,
                        crate::extensions::notification::SessionUpdate::TurnCompleted {
                            prompt_id: terminal_prompt_id, ..
                        } if terminal_prompt_id == prompt_id) => Some(*notification),
            super::super::storage::SessionUpdate::Acp(_)
            | super::super::storage::SessionUpdate::Xai(_) => None,
        })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "WakeCancelWire", into = "WakeCancelWire")]
pub struct WakeCancelRequest {
    pub context: PublicationContext,
    pub prompt_id: String,
    pub cancel_subagents: bool,
    pub trigger: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WakeCancelWire {
    session_id: String,
    context: PublicationContext,
    prompt_id: String,
    cancel_subagents: bool,
    trigger: Option<String>,
}

impl From<WakeCancelRequest> for WakeCancelWire {
    fn from(request: WakeCancelRequest) -> Self {
        Self {
            session_id: request.context.binding.session_id.clone(),
            context: request.context,
            prompt_id: request.prompt_id,
            cancel_subagents: request.cancel_subagents,
            trigger: request.trigger,
        }
    }
}

impl TryFrom<WakeCancelWire> for WakeCancelRequest {
    type Error = &'static str;

    fn try_from(wire: WakeCancelWire) -> Result<Self, Self::Error> {
        if wire.session_id != wire.context.binding.session_id {
            return Err("cancel settlement sessionId does not match binding");
        }
        Ok(Self {
            context: wire.context,
            prompt_id: wire.prompt_id,
            cancel_subagents: wire.cancel_subagents,
            trigger: wire.trigger,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WakeCancelSettlement {
    RemovedFromQueue,
    Terminal { notification: SessionNotification },
    Unresolved { reason: WakeCancelFailure },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WakeCancelFailure {
    SourceChanged,
    SessionUnavailable,
    ActorUnavailable,
    PersistenceUnavailable,
    UnknownTarget,
    InvalidTarget,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeCancelResponse {
    pub context: PublicationContext,
    pub prompt_id: String,
    pub settlement: WakeCancelSettlement,
}

#[derive(Debug, Clone)]
pub enum CancelTarget {
    Prompt(String),
    Wake {
        request: WakeCancelRequest,
        respond_to: tokio::sync::mpsc::UnboundedSender<WakeCancelResponse>,
    },
}

impl std::ops::Deref for CancelTarget {
    type Target = str;

    fn deref(&self) -> &str {
        match self {
            Self::Prompt(prompt_id) => prompt_id,
            Self::Wake { request, .. } => &request.prompt_id,
        }
    }
}
