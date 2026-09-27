use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum SubagentActor {
    Human,
    ParentModel { session_id: String },
    ParentTurn { prompt_id: String },
    SessionStop { session_id: String },
    ProcessRestart,
    Runtime,
    Limit { limit: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum InterruptionCause {
    ExplicitStop { actor: SubagentActor },
    Paused { actor: SubagentActor },
    ParentTurnCancelled { prompt_id: String },
    SessionStopped { session_id: String },
    SessionTeardown { session_id: String },
    ProcessRestart,
    LiveParentOrphan,
    Error { message: String },
    Limit { actor: SubagentActor, limit: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SubagentState {
    Running,
    Completed,
    Failed { message: String },
    Interrupted { cause: InterruptionCause },
}

impl SubagentState {
    pub const fn legacy_running() -> Self {
        Self::Running
    }

    pub const fn legacy_status(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed { .. } => "failed",
            Self::Interrupted { .. } => "cancelled",
        }
    }

    pub fn interruption(&self) -> Option<&InterruptionCause> {
        match self {
            Self::Interrupted { cause } => Some(cause),
            Self::Running | Self::Completed | Self::Failed { .. } => None,
        }
    }

    pub fn parse_legacy(status: &str, error: Option<&str>) -> Self {
        match status {
            "running" | "initializing" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed {
                message: error.unwrap_or("Subagent failed").to_owned(),
            },
            "cancelled" => Self::Interrupted {
                cause: InterruptionCause::ProcessRestart,
            },
            _ => Self::Failed {
                message: format!("Unrecognized legacy subagent state: {status}"),
            },
        }
    }
}

impl std::fmt::Display for InterruptionCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExplicitStop { actor } => write!(f, "stopped by {}", actor.label()),
            Self::Paused { actor } => write!(f, "paused by {}", actor.label()),
            Self::ParentTurnCancelled { prompt_id } => {
                write!(f, "parent turn {prompt_id} cancelled")
            }
            Self::SessionStopped { session_id } => write!(f, "session {session_id} stopped"),
            Self::SessionTeardown { session_id } => {
                write!(f, "parent session {session_id} ended")
            }
            Self::ProcessRestart => f.write_str("interrupted by process restart"),
            Self::LiveParentOrphan => f.write_str("orphaned while parent session stayed live"),
            Self::Error { message } => write!(f, "interrupted by error: {message}"),
            Self::Limit { actor, limit } => {
                write!(f, "stopped by {} at limit {limit}", actor.label())
            }
        }
    }
}

impl SubagentActor {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Human => "human user",
            Self::ParentModel { .. } => "parent model",
            Self::ParentTurn { .. } => "parent turn",
            Self::SessionStop { .. } => "session stop",
            Self::ProcessRestart => "process restart",
            Self::Runtime => "runtime",
            Self::Limit { .. } => "limit",
        }
    }
}
