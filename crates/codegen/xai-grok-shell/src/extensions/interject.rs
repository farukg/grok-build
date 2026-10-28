//! `x.ai/interject` extension handler.
//!
//! Queues a mid-turn interjection into the active session's pending interjection buffer.
//! The session actor drains it at the next safe point in `process_conversation_turn`.

use agent_client_protocol as acp;

use super::{ExtResult, parse_params};
use crate::agent::MvpAgent;
use crate::session::SessionCommand;

pub const INTERJECT_METHOD: &str = "x.ai/interject";

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterjectRequest {
    pub session_id: String,
    pub text: String,
    #[serde(default)]
    pub interjection_id: Option<String>,
    /// Optional structured blocks (text and images) from image-capable clients.
    /// Absent means the legacy text-only wire shape (empty after default).
    #[serde(default)]
    pub content: Vec<acp::ContentBlock>,
}

/// Handle `x.ai/interject`: queue a mid-turn user interjection.
pub async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let req: InterjectRequest = parse_params(args)?;
    let sid: acp::SessionId = req.session_id.clone().into();
    // An interjection racing a reconnect-replayed `session/load` (leader restart) waits for the load instead of failing
    let session_handle = agent.session_handle_waiting_for_load(&sid).await;
    let Some(session) = session_handle else {
        return Err(
            acp::Error::invalid_params().data(format!("session not found: {}", req.session_id))
        );
    };

    let (text_override, images) = super::content::split_content(req.content);
    let _ = session.cmd_tx.send(SessionCommand::Interject {
        text: text_override.unwrap_or(req.text),
        id: req.interjection_id,
        images,
    });

    super::to_ext_response(Ok(serde_json::json!({
        "status": "queued",
    })))
}

pub const QUEUE_INTERJECT_METHOD: &str = "x.ai/queue/interject";
pub const MAX_QUEUE_REQUEST_BYTES: usize = crate::leader::protocol::MAX_MESSAGE_SIZE as usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueCaller {
    session_id: acp::SessionId,
}

impl QueueCaller {
    pub(crate) fn from_session_control(session: &crate::session::SessionHandle) -> Self {
        Self { session_id: session.info.id.clone() }
    }

    pub(crate) fn controls(&self, session_id: &acp::SessionId) -> bool {
        self.session_id == *session_id
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.session_id.0.len()
    }

    #[cfg(test)]
    pub(crate) fn for_test_session(session_id: &str) -> Self {
        Self { session_id: acp::SessionId::new(session_id.to_owned()) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueOperation {
    pub session_id: String,
    pub id: String,
    pub expected_version: u64,
    pub operation_id: uuid::Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum QueueAction {
    Prioritize {
        #[serde(rename = "newText")]
        new_text: Option<String>,
    },
    Observe,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueueRequest {
    #[serde(flatten)]
    pub operation: QueueOperation,
    #[serde(flatten)]
    pub action: QueueAction,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueRejection {
    WrongSession,
    Running,
    Missing,
    Protected,
    Stale,
    NotOwner,
    OperationConflict,
    Unauthorized,
    Capacity,
    PayloadTooLarge,
}

impl QueueRejection {
    pub fn notice(&self) -> &'static str {
        match self {
            Self::WrongSession => "send now rejected: wrong session",
            Self::Running => "send now rejected: row already running",
            Self::Missing => "send now rejected: row no longer queued",
            Self::Protected => "send now rejected: protected origin",
            Self::Stale => "send now rejected: queue version changed",
            Self::NotOwner => "send now rejected: queue owner changed",
            Self::OperationConflict => "send now rejected: operation identity conflict",
            Self::Unauthorized => "send now rejected: session control not granted",
            Self::Capacity => "send now rejected: operation receipt capacity reached; existing operations remain observable",
            Self::PayloadTooLarge => "send now rejected: request exceeds the 64 MiB ACP limit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum QueueResult {
    Queued { row: crate::session::prompt_queue::QueueEntryWire },
    GoalInterjection { running_prompt_id: String },
    Started { prompt_id: String },
    Rejected {
        reason: QueueRejection,
        row: Option<crate::session::prompt_queue::QueueEntryWire>,
    },
    Unconfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueueResponse {
    pub operation: QueueOperation,
    pub result: QueueResult,
}

pub(crate) fn validate_queue_request_size(raw: &str) -> Result<(), acp::Error> {
    if raw.len() > MAX_QUEUE_REQUEST_BYTES {
        return Err(acp::Error::invalid_params().data(format!(
            "queue request exceeds {MAX_QUEUE_REQUEST_BYTES} protocol bytes; no operation admitted"
        )));
    }
    Ok(())
}

pub async fn handle_queue(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    validate_queue_request_size(args.params.get())?;
    let request: QueueRequest = parse_params(args)?;
    let sid = acp::SessionId::new(request.operation.session_id.clone());
    let Some(session) = agent.session_handle_waiting_for_load(&sid).await else {
        return super::to_raw_response(&QueueResponse {
            operation: request.operation,
            result: QueueResult::Unconfirmed,
        });
    };
    let caller = QueueCaller::from_session_control(&session);
    let operation = request.operation.clone();
    let (respond_to, receive) = tokio::sync::oneshot::channel();
    let response = match session.cmd_tx.send(SessionCommand::QueueSendNow { caller, request, respond_to }) {
        Ok(()) => receive.await.unwrap_or(QueueResponse {
            operation,
            result: QueueResult::Unconfirmed,
        }),
        Err(_) => QueueResponse { operation, result: QueueResult::Unconfirmed },
    };
    super::to_raw_response(&response)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Legacy wire shape (no `content`) parses byte-identically: text-only, zero images, no text override.
    #[test]
    fn parse_without_content_is_legacy_text_only() {
        let req: InterjectRequest = serde_json::from_value(serde_json::json!({
            "sessionId": "s1",
            "text": "steer left",
            "interjectionId": "i1",
        }))
        .expect("legacy params must parse");
        assert_eq!(req.text, "steer left");
        assert_eq!(req.interjection_id.as_deref(), Some("i1"));
        let (text_override, images) = super::super::content::split_content(req.content);
        assert_eq!(text_override, None);
        assert!(images.is_empty());
    }

    /// `content` with text and image blocks parses; the images are extracted.
    /// The Text block (the client's rewritten, path-stripped text) overrides the raw `text` param.
    #[test]
    fn parse_with_content_extracts_images_and_prefers_block_text() {
        let req: InterjectRequest = serde_json::from_value(serde_json::json!({
            "sessionId": "s1",
            "text": "look at [Image #1: /tmp/x.png]",
            "content": [
                { "type": "text", "text": "look at [Image #1]" },
                { "type": "image", "data": "aGVsbG8=", "mimeType": "image/png" },
            ],
        }))
        .expect("content params must parse");
        let (text_override, images) = super::super::content::split_content(req.content);
        assert_eq!(
            text_override.as_deref(),
            Some("look at [Image #1]"),
            "rewritten block text must win over the raw text param"
        );
        assert_eq!(images.len(), 1);
        let Some(image) = images.first() else {
            panic!("expected one image: {images:?}");
        };
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.data, "aGVsbG8=");
    }

    /// Garbage `content` fails the whole parse (strict, like other params) instead of silently dropping attachments.
    #[test]
    fn parse_with_garbage_content_is_an_error() {
        let result: Result<InterjectRequest, _> = serde_json::from_value(serde_json::json!({
            "sessionId": "s1",
            "text": "steer",
            "content": "not an array",
        }));
        assert!(result.is_err(), "garbage content must be rejected");
    }
}
