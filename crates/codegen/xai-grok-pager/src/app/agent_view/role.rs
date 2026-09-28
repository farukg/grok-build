use crate::app::agent::AgentId;
use agent_client_protocol as acp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentRole {
    Root,
    Child(ChildLink),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildLink {
    pub(crate) parent: AgentId,
    pub(crate) parent_session_id: acp::SessionId,
    pub(crate) subagent_id: String,
    pub(crate) started_at: std::time::Instant,
}
