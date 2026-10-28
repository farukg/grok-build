use std::time::Duration;

use tokio::time::Instant;
use crate::{ClockUnavailable, MonotonicPoint};
use crate::native_clock::ClockSample;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalClockMapping {
    /// Includes both calibration brackets and one millisecond of Linux Tokio timer rounding.
    /// This does not bound executor wake or dispatch latency.
    Correlated { earliest_at: Instant, latest_at: Instant },
    Uncorrelated,
}

impl MonotonicPoint {
    pub fn map_local(&self) -> LocalClockMapping {
        match self {
            Self::LinuxMonotonic { .. } => match ClockSample::read() {
                Ok(sample) => self.map_sample(&sample),
                Err(_) => LocalClockMapping::Uncorrelated,
            },
            Self::Unavailable { .. } => LocalClockMapping::Uncorrelated,
        }
    }

    fn map_sample(&self, sample: &ClockSample) -> LocalClockMapping {
        match self {
            Self::LinuxMonotonic { domain, earliest_ns, latest_ns } => {
                if domain != &sample.domain || earliest_ns > latest_ns {
                    return LocalClockMapping::Uncorrelated;
                }
                let earliest = sample.local_at(*earliest_ns, sample.after_ns);
                let latest = sample.local_at(*latest_ns, sample.before_ns)
                    .and_then(|at| at.checked_add(Duration::from_millis(1)));
                match earliest.zip(latest) {
                    Some((earliest_at, latest_at)) => LocalClockMapping::Correlated { earliest_at, latest_at },
                    None => LocalClockMapping::Uncorrelated,
                }
            }
            Self::Unavailable { .. } => LocalClockMapping::Uncorrelated,
        }
    }

    /// Projects the supplied anchor, never a replacement receipt/finish-time anchor.
    pub fn at(at: Instant) -> Self {
        match ClockSample::read().and_then(|sample| Self::from_sample(at, sample)) {
            Ok(clock) => clock,
            Err(reason) => Self::Unavailable { reason },
        }
    }

    fn from_sample(at: Instant, sample: ClockSample) -> Result<Self, ClockUnavailable> {
        let offset_ns = signed_duration_ns(at.into_std(), sample.local)?;
        let earliest_ns = sample.before_ns.checked_add(offset_ns).ok_or(ClockUnavailable::ClockRange)?;
        let latest_ns = sample.after_ns.checked_add(offset_ns).ok_or(ClockUnavailable::ClockRange)?;
        Ok(Self::LinuxMonotonic { domain: sample.domain, earliest_ns, latest_ns })
    }
}

fn signed_duration_ns(at: std::time::Instant, origin: std::time::Instant) -> Result<i64, ClockUnavailable> {
    match at.checked_duration_since(origin) {
        Some(offset) => i64::try_from(offset.as_nanos()).map_err(|_| ClockUnavailable::ClockRange),
        None => i64::try_from(origin.duration_since(at).as_nanos()).ok()
            .and_then(i64::checked_neg).ok_or(ClockUnavailable::ClockRange),
    }
}

#[cfg(test)]
#[path = "mapping_tests.rs"]
mod tests;
