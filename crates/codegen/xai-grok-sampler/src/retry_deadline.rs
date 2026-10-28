use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use xai_monotonic_clock::{LocalClockMapping, MonotonicPoint};
pub use xai_grok_sampling_types::error::RetryScheduleCause;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryDeadlineProjection {
    pub clock: MonotonicPoint,
    /// Diagnostic wall projection only; never sufficient evidence for a live countdown.
    pub wall_deadline_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClockMapping {
    Correlated { earliest_due_at: Instant, latest_due_at: Instant },
    Uncorrelated,
}

impl RetryDeadlineProjection {
    pub fn map_local(&self) -> RetryClockMapping {
        match self.clock.map_local() {
            LocalClockMapping::Correlated { earliest_at, latest_at } => RetryClockMapping::Correlated {
                earliest_due_at: earliest_at, latest_due_at: latest_at,
            },
            LocalClockMapping::Uncorrelated => RetryClockMapping::Uncorrelated,
        }
    }
}

/// The scheduler owns this anchor before any asynchronous notification delivery.
#[derive(Debug, Clone)]
pub struct RetryDeadline {
    checked: tokio::time::CheckedDeadline,
    projection: RetryDeadlineProjection,
}

impl RetryDeadline {
    pub fn after(backoff: Duration) -> Result<Self, RetryScheduleCause> {
        let at = Instant::now().checked_add(backoff).ok_or(RetryScheduleCause::InstantRange)?;
        let checked = tokio::time::CheckedDeadline::new(at).map_err(RetryScheduleCause::Deadline)?;
        let clock = MonotonicPoint::at(at);
        let wall_deadline_ms = i64::try_from(backoff.as_millis()).ok()
            .and_then(|ms| chrono::Utc::now().timestamp_millis().checked_add(ms));
        Ok(Self { checked, projection: RetryDeadlineProjection { clock, wall_deadline_ms } })
    }

    pub fn projection(&self) -> RetryDeadlineProjection {
        self.projection.clone()
    }

    pub fn remaining(&self) -> Duration {
        self.checked.deadline().saturating_duration_since(Instant::now())
    }

    pub async fn wait(&self) -> Result<(), tokio::time::error::Error> {
        self.checked.wait().await
    }
}

#[cfg(test)]
#[path = "retry_deadline_tests.rs"]
mod tests;
