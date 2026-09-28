use agent_client_protocol as acp;
use xai_acp_lib::acp_send;

pub(super) async fn remove_context_items(
    session_id: acp::SessionId,
    items: Vec<xai_grok_shell::session::ContextItemRef>,
    tx: &xai_acp_lib::AcpAgentTx,
) -> crate::app::actions::TaskResult {
    use crate::app::actions::TaskResult;
    let params = xai_grok_shell::extensions::remove_items::RemoveItemsSessionRequest {
        session_id: session_id.0.to_string(),
        items,
    };
    let raw = match serde_json::value::to_raw_value(&params) {
        Ok(raw) => raw,
        Err(error) => {
            tracing::warn!("Failed to encode remove-items request: {error}");
            return TaskResult::RemoveContextItemsComplete { session_id, outcome: Err(error.to_string()) };
        }
    };
    match acp_send(acp::ExtRequest::new("x.ai/session/remove_items", raw.into()), tx).await {
        Ok(response) => TaskResult::RemoveContextItemsComplete {
            session_id,
            outcome: serde_json::from_str::<xai_grok_shell::extensions::remove_items::RemoveItemsSessionResponse>(response.0.get())
                .ok()
                .map(|response| response.outcome)
                .ok_or_else(|| "invalid remove-items response".to_owned()),
        },
        Err(error) => {
            tracing::warn!("Failed to remove context items: {error}");
            TaskResult::RemoveContextItemsComplete { session_id, outcome: Err(error.to_string()) }
        }
    }
}
