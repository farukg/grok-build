//! Portable monotonic wire points. Native correlation is opt-in and performs no startup work.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxClockDomain {
    boot_id: Uuid,
    namespace_device: u64,
    namespace_inode: u64,
    monotonic_offset_ns: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockUnavailable {
    UnsupportedPlatform,
    DomainUnavailable,
    ClockUnavailable,
    ClockRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "clock", rename_all = "snake_case", try_from = "WirePoint")]
pub enum MonotonicPoint {
    LinuxMonotonic { domain: LinuxClockDomain, earliest_ns: i64, latest_ns: i64 },
    Unavailable { reason: ClockUnavailable },
}

#[derive(Deserialize)]
#[serde(tag = "clock", rename_all = "snake_case")]
enum WirePoint {
    LinuxMonotonic { domain: LinuxClockDomain, earliest_ns: i64, latest_ns: i64 },
    Unavailable { reason: ClockUnavailable },
}

impl TryFrom<WirePoint> for MonotonicPoint {
    type Error = &'static str;

    fn try_from(point: WirePoint) -> Result<Self, Self::Error> {
        match point {
            WirePoint::LinuxMonotonic { domain, earliest_ns, latest_ns } => {
                if domain.boot_id.is_nil() || earliest_ns > latest_ns {
                    return Err("invalid monotonic clock domain or interval");
                }
                Ok(Self::LinuxMonotonic { domain, earliest_ns, latest_ns })
            }
            WirePoint::Unavailable { reason } => Ok(Self::Unavailable { reason }),
        }
    }
}

impl Default for MonotonicPoint {
    fn default() -> Self {
        Self::Unavailable { reason: ClockUnavailable::ClockUnavailable }
    }
}

#[cfg(feature = "native")]
mod native_clock;
#[cfg(feature = "native")]
mod mapping;
#[cfg(feature = "native")]
pub use mapping::LocalClockMapping;

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
