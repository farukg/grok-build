//! Leader restart policy: classify how long ago the previous leader run stopped,
//! and persist the leader heartbeat that the classification reads.
//!
//! The heartbeat is a single RFC3339 wall-clock timestamp written next to the
//! leader lock. It is refreshed periodically by the leader server loop and once
//! more on orderly shutdown, so a SIGKILL still leaves the last periodic beat.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use xai_grok_tools::types::restart_stop_age::RestartStopAge;

/// Stopped longer than this ⇒ previously running obligations are cancelled at
/// leader startup. Exactly one hour is still fresh.
pub const STALE_AFTER: chrono::Duration = chrono::Duration::seconds(3600);

/// How often the leader server loop refreshes the heartbeat. Far below
/// [`STALE_AFTER`] so a leader that dies between beats is still classified fresh.
pub const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(600);

/// The heartbeat file paired with a leader lock path
/// (`/x/leader-foo.lock` becomes `/x/leader-foo.heartbeat`).
pub fn heartbeat_path(lock_path: &Path) -> PathBuf {
    lock_path.with_extension("heartbeat")
}

/// Classify a recorded stop instant against `now`. Pure.
///
/// A timestamp at or after `now` (clock rollback) cannot prove freshness and
/// classifies as stale: the decision runs once at startup and errs toward
/// cancelling work the user last touched over an hour ago.
pub fn classify(recorded_at: DateTime<Utc>, now: DateTime<Utc>) -> RestartStopAge {
    let elapsed = now.signed_duration_since(recorded_at);
    if elapsed > STALE_AFTER {
        RestartStopAge::Stale
    } else {
        RestartStopAge::Fresh
    }
}

/// The stop-age classification plus why, for startup logging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordedStopAge {
    /// No heartbeat exists: a first run with no prior leader state.
    FirstRun,
    Fresh {
        recorded_at: DateTime<Utc>,
    },
    /// Older than the grace window, unreadable, or in the future.
    Stale {
        recorded_at: Option<DateTime<Utc>>,
    },
}

impl RecordedStopAge {
    pub fn stop_age(self) -> RestartStopAge {
        match self {
            RecordedStopAge::FirstRun | RecordedStopAge::Fresh { .. } => RestartStopAge::Fresh,
            RecordedStopAge::Stale { .. } => RestartStopAge::Stale,
        }
    }
}

/// Read the heartbeat and classify it. An unreadable or corrupt heartbeat is
/// stale: the cancel only happens at startup, so the safe reading wins.
pub fn read_stop_age(heartbeat: &Path, now: DateTime<Utc>) -> RecordedStopAge {
    let contents = match std::fs::read_to_string(heartbeat) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return RecordedStopAge::FirstRun;
        }
        Err(error) => {
            tracing::warn!(path = %heartbeat.display(), %error, "leader heartbeat unreadable");
            return RecordedStopAge::Stale { recorded_at: None };
        }
    };
    let recorded_at = DateTime::parse_from_rfc3339(contents.trim())
        .map(|stamp| stamp.with_timezone(&Utc))
        .map_err(|error| {
            tracing::warn!(path = %heartbeat.display(), %error, "leader heartbeat unparsable");
            error
        })
        .ok();
    let Some(recorded_at) = recorded_at else {
        return RecordedStopAge::Stale { recorded_at: None };
    };
    match classify(recorded_at, now) {
        RestartStopAge::Fresh => RecordedStopAge::Fresh { recorded_at },
        RestartStopAge::Stale => RecordedStopAge::Stale {
            recorded_at: Some(recorded_at),
        },
    }
}

/// Overwrite the heartbeat with `at`. The leader holds the lock, so it is the
/// only writer; a torn write classifies as stale on the next start.
pub fn write_heartbeat(heartbeat: &Path, at: DateTime<Utc>) -> std::io::Result<()> {
    std::fs::write(heartbeat, format!("{}\n", at.to_rfc3339()))
}

#[cfg(test)]
#[path = "restart_policy_tests.rs"]
mod tests;
