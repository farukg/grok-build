//! `x.ai/session/context_policy`: revision-bound Context choices owned by the leader.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};
use xai_grok_sampling_types::{CategoryTokens, ContextPolicy, ContextPolicyEdit, ContextPolicyMutation, ContextPolicySelection};

use super::{ExtResult, parse_params, to_raw_response};
use crate::agent::MvpAgent;
use crate::session::helpers::context_policy_store::ContextPolicyError;
use crate::session::SessionHandle;

pub const CONTEXT_POLICY_CHANGED: &str = "x.ai/session/context_policy/changed";

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ContextPolicyAction {
    Get,
    Set { policy: ContextPolicy },
    Edit { edit: ContextPolicyEdit },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPolicyRequest {
    pub session_id: String,
    #[serde(flatten)]
    pub action: ContextPolicyAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPolicyReport {
    pub policy: ContextPolicy,
    #[serde(default)]
    pub usage: Vec<CategoryTokens>,
    #[serde(default)]
    pub selection: Option<ContextPolicySelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum ContextPolicyFailure {
    StorageIo { kind: ContextIoKind, os_code: Option<i32> },
    DurabilityUnconfirmed { kind: ContextIoKind, os_code: Option<i32> },
    StorageWorkerLost,
    InvalidToml,
    EncodeToml,
    InvalidLegacyJson,
    UnsupportedVersion { version: u32 },
    Stale,
    OriginChanged,
    RevisionExhausted,
    NotLoaded,
    MailboxClosed,
    ReplyLost,
    SessionUnavailable,
    SessionReplyLost,
    Apply { failures: Vec<ContextPolicyApplyFailure>, durability: ContextDurability },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextIoKind {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    InvalidData,
    WouldBlock,
    StorageFull,
    ReadOnlyFilesystem,
    Other,
}

impl From<std::io::ErrorKind> for ContextIoKind {
    fn from(kind: std::io::ErrorKind) -> Self {
        match kind {
            std::io::ErrorKind::NotFound => Self::NotFound,
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            std::io::ErrorKind::AlreadyExists => Self::AlreadyExists,
            std::io::ErrorKind::InvalidData => Self::InvalidData,
            std::io::ErrorKind::WouldBlock => Self::WouldBlock,
            std::io::ErrorKind::StorageFull => Self::StorageFull,
            std::io::ErrorKind::ReadOnlyFilesystem => Self::ReadOnlyFilesystem,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "durability", rename_all = "snake_case")]
pub enum ContextDurability {
    Confirmed,
    Unconfirmed { kind: ContextIoKind, os_code: Option<i32> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPolicyApplyFailure {
    pub session_id: String,
    pub cause: ContextAdmissionFailure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextAdmissionFailure {
    MailboxClosed,
    ReplyLost,
}

impl From<&xai_chat_state::ContextPolicyAdmissionError> for ContextAdmissionFailure {
    fn from(error: &xai_chat_state::ContextPolicyAdmissionError) -> Self {
        match error {
            xai_chat_state::ContextPolicyAdmissionError::MailboxClosed(_) => Self::MailboxClosed,
            xai_chat_state::ContextPolicyAdmissionError::ReplyLost(_) => Self::ReplyLost,
        }
    }
}

impl From<&ContextPolicyError> for ContextPolicyFailure {
    fn from(error: &ContextPolicyError) -> Self {
        match error {
            ContextPolicyError::Io(error) => Self::StorageIo { kind: error.kind().into(), os_code: error.raw_os_error() },
            ContextPolicyError::Durability(error) => Self::DurabilityUnconfirmed { kind: error.kind().into(), os_code: error.raw_os_error() },
            ContextPolicyError::Worker(_) => Self::StorageWorkerLost,
            ContextPolicyError::Decode(_) => Self::InvalidToml,
            ContextPolicyError::Encode(_) => Self::EncodeToml,
            ContextPolicyError::Legacy(_) => Self::InvalidLegacyJson,
            ContextPolicyError::UnsupportedVersion(version) => Self::UnsupportedVersion { version: *version },
            ContextPolicyError::Stale => Self::Stale,
            ContextPolicyError::OriginChanged => Self::OriginChanged,
            ContextPolicyError::RevisionExhausted => Self::RevisionExhausted,
            ContextPolicyError::NotLoaded => Self::NotLoaded,
            ContextPolicyError::Admission(error) => match ContextAdmissionFailure::from(error) {
                ContextAdmissionFailure::MailboxClosed => Self::MailboxClosed,
                ContextAdmissionFailure::ReplyLost => Self::ReplyLost,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ContextPolicyResponse {
    Confirmed { #[serde(flatten)] report: ContextPolicyReport },
    Rejected { failure: ContextPolicyFailure, current: Option<ContextPolicyReport> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPolicyNotification {
    pub session_id: String,
    pub response: ContextPolicyResponse,
}

pub(crate) fn publish_context_policy(
    gateway: &xai_acp_lib::AcpAgentGatewaySender,
    notification: &ContextPolicyNotification,
) {
    match serde_json::value::to_raw_value(notification) {
        Ok(params) => gateway.forward_fire_and_forget(acp::ExtNotification::new(CONTEXT_POLICY_CHANGED, params.into())),
        Err(error) => tracing::error!(%error, "could not encode Context notification"),
    }
}

pub(crate) async fn confirmed_report(handle: &SessionHandle) -> Result<ContextPolicyReport, ContextPolicyError> {
    let store = handle.context_policy_binding.authority.read().await;
    let selection = store.selection(handle.info.id.0.as_ref(), &handle.context_policy_binding.incarnation)?;
    let (policy, usage) = tokio::join!(
        handle.chat_state_handle.get_context_policy(),
        handle.chat_state_handle.get_context_usage(),
    );
    Ok(ContextPolicyReport { policy: policy?, usage, selection: Some(selection) })
}

#[tracing::instrument(skip_all, fields(method = %args.method))]
pub(crate) async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let request: ContextPolicyRequest = parse_params(args)?;
    let id = acp::SessionId::new(request.session_id.as_str());
    let Some(handle) = agent.session_handle_waiting_for_load(&id).await else {
        return to_raw_response(&ContextPolicyResponse::Rejected {
            failure: ContextPolicyFailure::SessionUnavailable, current: None,
        });
    };
    let edit = match request.action {
        ContextPolicyAction::Get => {
            let (respond_to, response) = tokio::sync::oneshot::channel();
            if handle.cmd_tx.send(crate::session::SessionCommand::GetContextPolicy { respond_to }).is_err() {
                return to_raw_response(&ContextPolicyResponse::Rejected {
                    failure: ContextPolicyFailure::SessionUnavailable, current: None,
                });
            }
            let response = match response.await {
                Ok(Ok(report)) => ContextPolicyResponse::Confirmed { report },
                Ok(Err(error)) => ContextPolicyResponse::Rejected { failure: (&error).into(), current: None },
                Err(_) => ContextPolicyResponse::Rejected { failure: ContextPolicyFailure::SessionReplyLost, current: None },
            };
            if !agent.resident_handle(&id).is_some_and(|current| current.cmd_tx.same_channel(&handle.cmd_tx)) {
                let current = match agent.resident_handle(&id) {
                    Some(current) => confirmed_report(&current).await.ok(),
                    None => None,
                };
                let response = ContextPolicyResponse::Rejected { failure: ContextPolicyFailure::OriginChanged, current };
                publish_context_policy(&agent.gateway, &ContextPolicyNotification { session_id: id.0.to_string(), response: response.clone() });
                return to_raw_response(&response);
            }
            publish_context_policy(&agent.gateway, &ContextPolicyNotification { session_id: id.0.to_string(), response: response.clone() });
            return to_raw_response(&response);
        },
        ContextPolicyAction::Edit { edit } => edit,
        ContextPolicyAction::Set { policy } => {
            let version = {
                let store = handle.context_policy_binding.authority.read().await;
                store.selection(id.0.as_ref(), &handle.context_policy_binding.incarnation)
            };
            match version {
                Ok(selection) => ContextPolicyEdit {
                    expected: selection.version,
                    mutation: ContextPolicyMutation::ReplaceSession { policy },
                },
                Err(error) => return to_raw_response(&ContextPolicyResponse::Rejected {
                    failure: (&error).into(), current: None,
                }),
            }
        }
    };
    let result = agent.edit_context_policy(&id, &handle, &edit).await;
    let current = confirmed_report(&handle).await;
    let response = match (result, current) {
        (Ok(()), Ok(report)) => ContextPolicyResponse::Confirmed { report },
        (Err(failure), current) => ContextPolicyResponse::Rejected { failure, current: current.ok() },
        (Ok(()), Err(error)) => ContextPolicyResponse::Rejected { failure: (&error).into(), current: None },
    };
    to_raw_response(&response)
}
