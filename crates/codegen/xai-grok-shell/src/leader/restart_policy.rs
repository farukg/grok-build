//! Only an observed orderly stop supplies a stop timestamp.
//! A running marker after a crash, or a legacy heartbeat, is not a stop time.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use xai_grok_tools::types::restart_recovery::RestartStartup;
use xai_grok_tools::types::restart_stop_age::RestartStopAge;

pub const STALE_AFTER: chrono::Duration = chrono::Duration::seconds(3600);

pub fn heartbeat_path(lock_path: &Path) -> PathBuf {
    lock_path.with_extension("heartbeat")
}

pub fn classify(recorded_at: DateTime<Utc>, now: DateTime<Utc>) -> RestartStopAge {
    if now.signed_duration_since(recorded_at) > STALE_AFTER {
        RestartStopAge::Stale
    } else {
        RestartStopAge::Fresh
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordedStopAge {
    FirstRun,
    Unknown,
    Fresh { recorded_at: DateTime<Utc> },
    Stale { recorded_at: DateTime<Utc> },
}

impl RecordedStopAge {
    pub fn stop_age(self) -> RestartStopAge {
        match self {
            Self::FirstRun | Self::Unknown | Self::Fresh { .. } => RestartStopAge::Fresh,
            Self::Stale { .. } => RestartStopAge::Stale,
        }
    }
}

pub struct RestartEvidence {
    pub stop: RecordedStopAge,
    pub startup: RestartStartup,
}

pub fn read_evidence(heartbeat: &Path, now: DateTime<Utc>) -> std::io::Result<RestartEvidence> {
    let contents = match std::fs::read_to_string(heartbeat) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RestartEvidence { stop: RecordedStopAge::FirstRun, startup: RestartStartup::Continue });
        }
        Err(error) => {
            tracing::warn!(path = %heartbeat.display(), %error, "leader stop evidence unreadable");
            return Err(error);
        }
    };
    let mut lines = contents.lines();
    let stop = lines.next().and_then(|line| line.trim().strip_prefix("stopped "))
        .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
        .map(|stamp| stamp.with_timezone(&Utc))
        .filter(|stamp| *stamp <= now)
        .map_or(RecordedStopAge::Unknown, |recorded_at| match classify(recorded_at, now) {
            RestartStopAge::Fresh => RecordedStopAge::Fresh { recorded_at },
            RestartStopAge::Stale => RecordedStopAge::Stale { recorded_at },
        });
    let startup = match lines.next() {
        None => RestartStartup::Continue,
        Some(line) => {
            let encoded = line.strip_prefix("restart ").ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::InvalidData, "invalid persisted restart cohort",
            ))?;
            serde_json::from_str(encoded).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?
        }
    };
    Ok(RestartEvidence { stop, startup })
}

pub fn write_running_with_startup(heartbeat: &Path, startup: RestartStartup) -> std::io::Result<()> {
    write_record(heartbeat, "running".into(), startup)
}

pub fn write_stopped_with_startup(heartbeat: &Path, at: DateTime<Utc>, startup: RestartStartup) -> std::io::Result<()> {
    write_record(heartbeat, format!("stopped {}", at.to_rfc3339()), startup)
}

fn write_record(path: &Path, lifecycle: String, startup: RestartStartup) -> std::io::Result<()> {
    let startup = serde_json::to_string(&startup).map_err(std::io::Error::other)?;
    crate::session::storage::write_bytes_atomic(path, format!("{lifecycle}\nrestart {startup}\n").as_bytes())
}

#[cfg(test)]
fn read_stop_age(heartbeat: &Path, now: DateTime<Utc>) -> RecordedStopAge {
    read_evidence(heartbeat, now).map_or(RecordedStopAge::Unknown, |evidence| evidence.stop)
}

#[cfg(test)]
fn write_running(heartbeat: &Path) -> std::io::Result<()> {
    write_running_with_startup(heartbeat, RestartStartup::Continue)
}

#[cfg(test)]
fn write_heartbeat(heartbeat: &Path, at: DateTime<Utc>) -> std::io::Result<()> {
    write_stopped_with_startup(heartbeat, at, RestartStartup::Continue)
}

#[cfg(test)]
#[path = "restart_policy_tests.rs"]
mod tests;
