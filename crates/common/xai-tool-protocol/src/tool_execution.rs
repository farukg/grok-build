//! Dispatch timing, not call lifetime or an assertion about a detached backend process.
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub use xai_monotonic_clock::MonotonicPoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolExecutionId(Uuid);

impl ToolExecutionId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolExecutionTermination {
    /// Dispatch returned a typed result; its success/failure/cancellation outcome is separate.
    Returned,
    DispatchCancelled,
    StreamEndedWithoutTerminal,
}

/// Execution IDs distinguish actual dispatch attempts, including authentication retries.
/// No wall timestamp or receipt time is a live monotonic clock projection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ToolExecutionTiming {
    #[default]
    Unknown,
    Started {
        execution_id: ToolExecutionId,
        #[serde(default)]
        started_at: MonotonicPoint,
    },
    Finished {
        execution_id: ToolExecutionId,
        #[serde(default)]
        started_at: MonotonicPoint,
        duration_ms: u64,
        termination: ToolExecutionTermination,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ToolCallSource {
    #[default]
    Unknown,
    Sampling { prompt_id: String, request_id: String, attempt: u64 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ownership", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ToolCallOwnership {
    #[default]
    Unknown,
    Registered {
        invocation_id: String,
        provider_call_id: String,
        source: ToolCallSource,
    },
}

#[cfg(test)]
#[path = "tool_execution_tests.rs"]
mod tests;
