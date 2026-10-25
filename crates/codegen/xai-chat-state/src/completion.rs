use serde::{Deserialize, Deserializer, Serialize, Serializer};
use xai_grok_sampling_types::ConversationItem;
use xai_message_delivery_core::AttemptId;

use crate::StrictAppendError;

pub use xai_message_delivery_core::SubagentCompletionId;

pub mod completion_id_serde {
    use super::*;
    pub fn serialize<S: Serializer>(id: &SubagentCompletionId, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> { subagent_id: &'a str, attempt_id: &'a str }
        Wire { subagent_id: &id.subagent_id, attempt_id: id.attempt_id.as_str() }.serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SubagentCompletionId, D::Error> {
        #[derive(Deserialize)]
        struct Wire { subagent_id: String, attempt_id: String }
        let wire = Wire::deserialize(deserializer)?;
        let attempt_id = AttemptId::parse(&wire.attempt_id).ok_or_else(|| serde::de::Error::custom("invalid completion attempt"))?;
        Ok(SubagentCompletionId { subagent_id: wire.subagent_id, attempt_id })
    }
}

pub mod attempt_serde {
    use super::*;

    pub fn serialize<S: Serializer>(attempt: &AttemptId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(attempt.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<AttemptId, D::Error> {
        let token = String::deserialize(deserializer)?;
        AttemptId::parse(&token).ok_or_else(|| serde::de::Error::custom("invalid completion attempt"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionTaskKind { Bash, Monitor }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum TerminalCompletionId {
    Task { owner_session_id: String, task_id: String, task_kind: CompletionTaskKind },
    Workflow { run_id: String, revision: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletionVersion { #[serde(rename = "2")] V2 }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompletionIdentity {
    Subagent(#[serde(with = "completion_id_serde")] SubagentCompletionId),
    V2 { version: CompletionVersion, #[serde(flatten)] terminal: TerminalCompletionId },
}

impl From<SubagentCompletionId> for CompletionIdentity {
    fn from(id: SubagentCompletionId) -> Self { Self::Subagent(id) }
}
impl From<TerminalCompletionId> for CompletionIdentity {
    fn from(terminal: TerminalCompletionId) -> Self { Self::V2 { version: CompletionVersion::V2, terminal } }
}
impl PartialEq<SubagentCompletionId> for CompletionIdentity {
    fn eq(&self, other: &SubagentCompletionId) -> bool { matches!(self, Self::Subagent(id) if id == other) }
}
impl CompletionIdentity {
    pub fn subagent(&self) -> Option<&SubagentCompletionId> {
        match self { Self::Subagent(id) => Some(id), Self::V2 { .. } => None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionReceipt {
    pub completion: CompletionIdentity,
    pub prompt_id: String,
    #[serde(default)]
    pub parent_task: crate::ParentTask,
}

/// Receipt and model input share one durable JSONL append.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionProjection {
    #[serde(flatten)]
    pub item: ConversationItem,
    #[serde(rename = "_subagent_completion")]
    pub receipt: CompletionReceipt,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "_subagent_completion_additional")]
    pub additional_receipts: Vec<CompletionReceipt>,
}

#[derive(Clone, Debug)]
pub enum CompletionProjectionAck {
    Appended,
    Continue { history: Vec<ConversationItem> },
    TurnClosed,
    /// Also reconciles a caller whose persistence acknowledgement was lost.
    AlreadyPresent { history: Vec<ConversationItem> },
}

pub type CompletionProjectionError = StrictAppendError<CompletionProjectionAck>;

impl CompletionProjection {
    pub fn receipts(&self) -> impl Iterator<Item = &CompletionReceipt> {
        std::iter::once(&self.receipt).chain(&self.additional_receipts)
    }

    pub fn new(completion: impl Into<CompletionIdentity>, prompt_id: String, text: String) -> Self {
        Self {
            item: ConversationItem::subagent_completed(text),
            receipt: CompletionReceipt { completion: completion.into(), prompt_id, parent_task: crate::ParentTask::Unknown },
            additional_receipts: Vec::new(),
        }
    }
}
