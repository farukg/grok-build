use serde::{Deserialize, Serialize};

use super::types::{InterruptionCause, SubagentResult, SubagentState};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SubagentTerminalState {
    Completed,
    Failed { message: String },
    Interrupted { cause: InterruptionCause },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        let state = match &result.state {
            SubagentState::Running => return None,
            SubagentState::Completed => SubagentTerminalState::Completed,
            SubagentState::Failed { message } => SubagentTerminalState::Failed {
                message: message.clone(),
            },
            SubagentState::Interrupted { cause } => SubagentTerminalState::Interrupted {
                cause: cause.clone(),
            },
        };
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
