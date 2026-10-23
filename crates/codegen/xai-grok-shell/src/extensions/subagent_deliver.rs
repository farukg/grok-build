//! `x.ai/subagent/deliver` — explicitly set a running child's completion delivery policy.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};

use crate::agent::MvpAgent;
use crate::session::ExtMethodResult;

use super::ExtResult;
use super::notification::SubagentDelivery;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliverSubagentRequest {
    /// The child session id.
    pub session_id: String,
    /// Applies to this running attempt; omitted by older clients to release a hold.
    #[serde(default)]
    pub delivery: SubagentDelivery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliverSubagentOutcome {
    /// The answer goes to the caller once the running turn, if any, ends.
    Delivered,
    /// Nobody holds the child; it delivers when its turn ends anyway.
    NotHeld,
    /// Completion stays pending until automatic delivery is explicitly restored.
    Held,
    NotRunning,
}

pub(crate) async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let req: DeliverSubagentRequest = super::parse_params(args)?;
    let outcome = agent.deliver_child(&acp::SessionId::new(req.session_id), req.delivery);
    ExtMethodResult::from_result(Ok::<_, String>(outcome))
        .to_ext_response()
        .map_err(|e| acp::Error::internal_error().data(e.to_string()))
}
