//! `x.ai/session/context_policy`: read or replace which context categories a session sends.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use xai_grok_sampling_types::{CategoryTokens, ContextPolicy};

use super::{ExtResult, parse_params, to_raw_response};
use crate::agent::MvpAgent;
use crate::session::SessionCommand;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ContextPolicyAction {
    Get,
    Set { policy: ContextPolicy },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPolicyRequest {
    pub session_id: String,
    #[serde(flatten)]
    pub action: ContextPolicyAction,
}

/// The policy and what each category costs today.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPolicyReport {
    pub policy: ContextPolicy,
    #[serde(default)]
    pub usage: Vec<CategoryTokens>,
}

#[tracing::instrument(skip_all, fields(method = %args.method))]
pub(crate) async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let request: ContextPolicyRequest = parse_params(args)?;
    let id = acp::SessionId::new(request.session_id.as_str());
    let handle = agent
        .session_handle_waiting_for_load(&id)
        .await
        .ok_or_else(|| acp::Error::resource_not_found(Some("session not found".into())))?;
    let unavailable = || acp::Error::internal_error().data("session failed to respond");
    let report = match request.action {
        ContextPolicyAction::Get => {
            let (tx, rx) = oneshot::channel();
            handle
                .cmd_tx
                .send(SessionCommand::GetContextPolicy { respond_to: tx })
                .map_err(|_| unavailable())?;
            rx.await.map_err(|_| unavailable())?
        }
        ContextPolicyAction::Set { policy } => {
            let (tx, rx) = oneshot::channel();
            handle
                .cmd_tx
                .send(SessionCommand::SetContextPolicy {
                    policy,
                    respond_to: tx,
                })
                .map_err(|_| unavailable())?;
            rx.await.map_err(|_| unavailable())?
        }
    };
    to_raw_response(&report)
}
