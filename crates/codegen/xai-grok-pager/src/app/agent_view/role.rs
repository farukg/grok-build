use crate::app::agent::AgentId;
use crate::app::subagent::SubagentInfo;
use agent_client_protocol as acp;
use std::borrow::Cow;

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
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionKindLabel<'a> {
    Main,
    Subagent { kind: Cow<'a, str> },
}

impl<'a> SessionKindLabel<'a> {
    pub(crate) fn from_subagent(info: &'a SubagentInfo) -> Self {
        let (kind, _) = crate::app::subagent::format_subagent_label(info);
        Self::Subagent { kind: Cow::Owned(kind) }
    }
}
