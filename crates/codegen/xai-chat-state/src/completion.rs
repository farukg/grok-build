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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionReceipt {
    #[serde(with = "completion_id_serde")]
    pub completion: SubagentCompletionId,
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

    pub fn new(completion: SubagentCompletionId, prompt_id: String, text: String) -> Self {
        Self {
            item: ConversationItem::subagent_completed(text),
            receipt: CompletionReceipt { completion, prompt_id, parent_task: crate::ParentTask::Unknown },
            additional_receipts: Vec::new(),
        }
    }
}
