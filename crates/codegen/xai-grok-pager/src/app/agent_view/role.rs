use crate::app::agent::AgentId;
use agent_client_protocol as acp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentRole {
    Root,
    Child(ChildLink),
}

impl AgentRole {
    pub(crate) fn prompt_target(&self) -> PromptTarget {
        match self {
            Self::Root => PromptTarget::Session,
            Self::Child(link) => PromptTarget::ChildOf(link.parent_session_id.clone()),
        }
    }
}

/// Whom a prompt's session id names. A shell that lost a child session (restart) continues it
/// from its parent's record instead of refusing the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptTarget {
    Session,
    ChildOf(acp::SessionId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildLink {
    pub(crate) parent: AgentId,
    pub(crate) parent_session_id: acp::SessionId,
    pub(crate) subagent_id: String,
    pub(crate) started_at: std::time::Instant,
}
