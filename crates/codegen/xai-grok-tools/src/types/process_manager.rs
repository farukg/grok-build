//! Process manager utilities.
//!
//! The `ProcessManager` trait has been replaced by `TerminalBackend` in `computer/types.rs`.
//! This module retains the `format_system_time_rfc3339` utility and re-exports from types.

pub use crate::computer::types::{KillSource, TaskSnapshot};
pub use xai_tool_types::{KillFailureReason, KillOutcome, KillProcessEffect, KillStage};

/// Format a SystemTime as an RFC 3339 string.
pub fn format_system_time_rfc3339(time: std::time::SystemTime) -> String {
    use chrono::{DateTime, Utc};
    let datetime: DateTime<Utc> = time.into();
    datetime.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
