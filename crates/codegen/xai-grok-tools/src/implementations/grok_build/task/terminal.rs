use serde::{Deserialize, Serialize};

use super::types::{InterruptionCause, SubagentResult, SubagentState};
use super::inventory::{
    AttemptEvidence, ChildEvidenceSource, InventoryValidationError, validate_sources,
};
use super::types::SubagentSnapshotStatus;
use xai_chat_state::SubagentCompletionId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SubagentTerminalState {
    Completed,
    Failed { message: String },
    Interrupted { cause: InterruptionCause },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentTerminal {
    pub attempt_id: String,
    pub state: SubagentTerminalState,
    #[serde(serialize_with = "serialize_output", deserialize_with = "deserialize_output")]
    pub output: std::sync::Arc<str>,
    pub tool_calls: u32,
    pub turns: u32,
    pub duration_ms: u64,
    pub tokens_used: u64,
}

fn serialize_output<S: serde::Serializer>(output: &std::sync::Arc<str>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(output)
}

fn deserialize_output<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<std::sync::Arc<str>, D::Error> {
    String::deserialize(deserializer).map(std::sync::Arc::from)
}

impl SubagentTerminal {
    pub(super) fn from_result(
        attempt_id: &xai_message_delivery_core::AttemptId,
        result: &SubagentResult,
        persisted_output: Option<&str>,
    ) -> Option<Self> {
        let state = SubagentTerminalState::from_state(&result.state)?;
        Some(Self {
            attempt_id: attempt_id.to_string(),
            state,
            output: persisted_output.map(std::sync::Arc::from).unwrap_or_else(|| result.output.clone()),
            tool_calls: result.tool_calls,
            turns: result.turns,
            duration_ms: result.duration_ms,
            tokens_used: result.tokens_used,
        })
    }
}

impl SubagentTerminalState {
    pub fn from_state(state: &SubagentState) -> Option<Self> {
        match state {
            SubagentState::Running => None,
            SubagentState::Completed => Some(Self::Completed),
            SubagentState::Failed { message } => Some(Self::Failed { message: message.clone() }),
            SubagentState::Interrupted { cause } => Some(Self::Interrupted { cause: cause.clone() }),
        }
    }

    pub fn state(&self) -> SubagentState {
        match self {
            Self::Completed => SubagentState::Completed,
            Self::Failed { message } => SubagentState::Failed { message: message.clone() },
            Self::Interrupted { cause } => SubagentState::Interrupted { cause: cause.clone() },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecordedTerminalState {
    Completed,
    Failed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Cancelled {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cause: Option<InterruptionCause>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

impl RecordedTerminalState {
    pub fn state(&self) -> Option<SubagentState> {
        match self {
            Self::Completed => Some(SubagentState::Completed),
            Self::Failed { message } => message.as_ref().map(|message| SubagentState::Failed {
                message: message.clone(),
            }),
            Self::Cancelled { cause, .. } => cause.as_ref().map(|cause| SubagentState::Interrupted {
                cause: cause.clone(),
            }),
        }
    }

    pub fn legacy_status(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed { .. } => "failed",
            Self::Cancelled { .. } => "cancelled",
        }
    }

    pub(crate) fn matches_producer(&self, state: &SubagentTerminalState) -> bool {
        match self {
            Self::Completed => matches!(state, SubagentTerminalState::Completed),
            Self::Failed { message } => match state {
                SubagentTerminalState::Failed { message: actual } => {
                    message.as_ref().is_none_or(|message| message == actual)
                }
                SubagentTerminalState::Completed | SubagentTerminalState::Interrupted { .. } => false,
            },
            Self::Cancelled { cause, .. } => match state {
                SubagentTerminalState::Interrupted { cause: actual } => {
                    cause.as_ref().is_none_or(|cause| cause == actual)
                }
                SubagentTerminalState::Completed | SubagentTerminalState::Failed { .. } => false,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedSubagentTerminal {
    pub attempt: AttemptEvidence,
    pub state: RecordedTerminalState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_output",
        deserialize_with = "deserialize_optional_output"
    )]
    pub output: Option<std::sync::Arc<str>>,
    pub sources: Vec<ChildEvidenceSource>,
}

impl RecordedSubagentTerminal {
    pub fn validate(&self, subagent_id: &str) -> Result<(), InventoryValidationError> {
        validate_sources(&self.sources, subagent_id, &self.attempt)
    }

    fn snapshot_status(&self) -> Option<SubagentSnapshotStatus> {
        match &self.state {
            RecordedTerminalState::Completed => self.completed_snapshot_status(),
            RecordedTerminalState::Failed { message } => message.as_ref().map(|message| {
                SubagentSnapshotStatus::Failed { error: message.clone() }
            }),
            RecordedTerminalState::Cancelled { cause, .. } => cause.as_ref().map(|cause| {
                SubagentSnapshotStatus::Cancelled { cause: cause.clone() }
            }),
        }
    }

    fn completed_snapshot_status(&self) -> Option<SubagentSnapshotStatus> {
        Some(SubagentSnapshotStatus::Completed {
            output: self.output.as_ref()?.to_string(),
            tool_calls: self.tool_calls?,
            turns: self.turns?,
            worktree_path: None,
        })
    }
}

fn serialize_optional_output<S: serde::Serializer>(
    output: &Option<std::sync::Arc<str>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    output.as_deref().serialize(serializer)
}

fn deserialize_optional_output<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<std::sync::Arc<str>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(|output| output.map(std::sync::Arc::from))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "evidence", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ChildTerminalEvidence {
    Producer {
        #[serde(with = "xai_chat_state::completion::completion_id_serde")]
        completion_id: SubagentCompletionId,
        result: SubagentTerminal,
        sources: Vec<ChildEvidenceSource>,
    },
    ProducerFact {
        #[serde(with = "xai_chat_state::completion::completion_id_serde")]
        completion_id: SubagentCompletionId,
        state: SubagentTerminalState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recorded: Option<RecordedSubagentTerminal>,
        sources: Vec<ChildEvidenceSource>,
    },
    Recorded { result: RecordedSubagentTerminal },
}

impl ChildTerminalEvidence {
    pub fn attempt(&self) -> AttemptEvidence {
        match self {
            Self::Producer { completion_id, .. } | Self::ProducerFact { completion_id, .. } => {
                AttemptEvidence::Known(completion_id.attempt_id.clone())
            }
            Self::Recorded { result } => result.attempt.clone(),
        }
    }

    pub fn state(&self) -> Option<SubagentState> {
        match self {
            Self::Producer { result, .. } => Some(result.state.state()),
            Self::ProducerFact { state, .. } => Some(state.state()),
            Self::Recorded { result } => result.state.state(),
        }
    }

    pub fn duration_ms(&self) -> Option<u64> {
        match self {
            Self::Producer { result, .. } => Some(result.duration_ms),
            Self::ProducerFact { recorded, .. } => recorded.as_ref().and_then(|result| result.duration_ms),
            Self::Recorded { result } => result.duration_ms,
        }
    }

    pub fn legacy_status(&self) -> &'static str {
        match self {
            Self::Producer { result, .. } => result.state.state().legacy_status(),
            Self::ProducerFact { state, .. } => state.state().legacy_status(),
            Self::Recorded { result } => result.state.legacy_status(),
        }
    }

    pub fn sources(&self) -> &[ChildEvidenceSource] {
        match self {
            Self::Producer { sources, .. } | Self::ProducerFact { sources, .. } => sources,
            Self::Recorded { result } => &result.sources,
        }
    }

    pub fn recorded(&self) -> Option<&RecordedSubagentTerminal> {
        match self {
            Self::Producer { .. } => None,
            Self::ProducerFact { recorded, .. } => recorded.as_ref(),
            Self::Recorded { result } => Some(result),
        }
    }

    pub fn validate(
        &self,
        subagent_id: &str,
        attempt: &AttemptEvidence,
    ) -> Result<(), InventoryValidationError> {
        if &self.attempt() != attempt {
            return Err(InventoryValidationError::AttemptMismatch);
        }
        validate_sources(self.sources(), subagent_id, attempt)?;
        match self {
            Self::Producer { completion_id, result, .. } => {
                if completion_id.subagent_id != subagent_id {
                    return Err(InventoryValidationError::IdentityMismatch);
                }
                if result.attempt_id != completion_id.attempt_id.as_str() {
                    return Err(InventoryValidationError::AttemptMismatch);
                }
            }
            Self::ProducerFact { completion_id, state, recorded, .. } => {
                if completion_id.subagent_id != subagent_id {
                    return Err(InventoryValidationError::IdentityMismatch);
                }
                if let Some(recorded) = recorded {
                    recorded.validate(subagent_id)?;
                    if &recorded.attempt != attempt {
                        return Err(InventoryValidationError::AttemptMismatch);
                    }
                    if !recorded.state.matches_producer(state) {
                        return Err(InventoryValidationError::InvalidEvidence);
                    }
                }
            }
            Self::Recorded { result } => result.validate(subagent_id)?,
        }
        Ok(())
    }

    pub(super) fn snapshot_status(&self) -> Option<SubagentSnapshotStatus> {
        match self {
            Self::Producer { result, .. } => Some(match &result.state {
                SubagentTerminalState::Completed => SubagentSnapshotStatus::Completed {
                    output: result.output.to_string(),
                    tool_calls: result.tool_calls,
                    turns: result.turns,
                    worktree_path: None,
                },
                SubagentTerminalState::Failed { message } => SubagentSnapshotStatus::Failed {
                    error: message.clone(),
                },
                SubagentTerminalState::Interrupted { cause } => SubagentSnapshotStatus::Cancelled {
                    cause: cause.clone(),
                },
            }),
            Self::ProducerFact { state, recorded, .. } => match state {
                SubagentTerminalState::Completed => recorded.as_ref()?.completed_snapshot_status(),
                SubagentTerminalState::Failed { message } => Some(SubagentSnapshotStatus::Failed {
                    error: message.clone(),
                }),
                SubagentTerminalState::Interrupted { cause } => Some(SubagentSnapshotStatus::Cancelled {
                    cause: cause.clone(),
                }),
            },
            Self::Recorded { result } => result.snapshot_status(),
        }
    }
}
