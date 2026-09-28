//! `x.ai/session/remove_items`: remove model-context items from a live session.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use super::{ExtResult, parse_params, to_raw_response};
use crate::agent::MvpAgent;
use crate::session::{RemoveContextItemsOutcome, RemoveItemsRequest, SessionCommand};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveItemsSessionRequest {
    pub session_id: String,
    pub items: Vec<crate::session::ContextItemRef>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveItemsSessionResponse {
    pub outcome: RemoveContextItemsOutcome,
}

#[tracing::instrument(skip_all, fields(method = %args.method))]
pub(crate) async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let request: RemoveItemsSessionRequest = parse_params(args)?;
    let id = acp::SessionId::new(request.session_id.as_str());
    let handle = agent
        .session_handle_waiting_for_load(&id)
        .await
        .ok_or_else(|| acp::Error::resource_not_found(Some("session not found".into())))?;
    let (tx, rx) = oneshot::channel();
    handle
        .cmd_tx
        .send(SessionCommand::RemoveContextItems {
            request: RemoveItemsRequest { items: request.items },
            respond_to: tx,
        })
        .map_err(|_| acp::Error::internal_error().data("failed to send remove-items command"))?;
    let outcome = rx
        .await
        .map_err(|_| acp::Error::internal_error().data("session failed to respond"))?;
    to_raw_response(&RemoveItemsSessionResponse { outcome })
}
