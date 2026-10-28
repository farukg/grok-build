use std::time::Instant as StdInstant;

use tokio::time::Instant;
#[cfg(target_os = "linux")]
use uuid::Uuid;

use crate::{ClockUnavailable, LinuxClockDomain};

pub(super) struct ClockSample {
    pub(super) domain: LinuxClockDomain,
    pub(super) local: StdInstant,
    pub(super) before_ns: i64,
    pub(super) after_ns: i64,
}

impl ClockSample {
    #[cfg(target_os = "linux")]
    pub(super) fn read() -> Result<Self, ClockUnavailable> {
        let domain = LinuxClockDomain::read()?;
        // Linux std::Instant and CLOCK_MONOTONIC exclude suspend and share clock adjustments.
        let before_ns = monotonic_ns()?;
        let local = StdInstant::now();
        let after_ns = monotonic_ns()?;
        if before_ns > after_ns {
            return Err(ClockUnavailable::ClockUnavailable);
        }
        Ok(Self {
            domain,
            local,
            before_ns,
            after_ns,
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn read() -> Result<Self, ClockUnavailable> {
        Err(ClockUnavailable::UnsupportedPlatform)
    }

    pub(super) fn local_at(&self, deadline_ns: i64, native_ns: i64) -> Option<Instant> {
        let offset_ns = deadline_ns.checked_sub(native_ns)?;
        let offset = std::time::Duration::from_nanos(offset_ns.unsigned_abs());
        let local = Instant::from_std(self.local);
        if offset_ns >= 0 {
            local.checked_add(offset)
        } else {
            local.checked_sub(offset)
        }
    }
}

#[cfg(target_os = "linux")]
impl LinuxClockDomain {
    fn read() -> Result<Self, ClockUnavailable> {
        use std::os::unix::fs::MetadataExt;

        let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|_| ClockUnavailable::DomainUnavailable)?;
        let boot_id =
            Uuid::parse_str(boot_id.trim()).map_err(|_| ClockUnavailable::DomainUnavailable)?;
        if boot_id.is_nil() {
            return Err(ClockUnavailable::DomainUnavailable);
        }
        let namespace = std::fs::metadata("/proc/thread-self/ns/time")
            .map_err(|_| ClockUnavailable::DomainUnavailable)?;
        let children = std::fs::metadata("/proc/thread-self/ns/time_for_children")
            .map_err(|_| ClockUnavailable::DomainUnavailable)?;
        // timens_offsets describes time_for_children, which may differ after unshare/setns.
        if namespace.dev() != children.dev() || namespace.ino() != children.ino() {
            return Err(ClockUnavailable::DomainUnavailable);
        }
        let offsets = std::fs::read_to_string("/proc/thread-self/timens_offsets")
            .map_err(|_| ClockUnavailable::DomainUnavailable)?;
        // Offsets also distinguish namespace-inode reuse within one boot.
        let monotonic_offset_ns = monotonic_offset_ns(&offsets)?;
        Ok(Self {
            boot_id,
            namespace_device: namespace.dev(),
            namespace_inode: namespace.ino(),
            monotonic_offset_ns,
        })
    }
}

#[cfg(target_os = "linux")]
fn monotonic_ns() -> Result<i64, ClockUnavailable> {
    use nix::time::{ClockId, clock_gettime};

    let at =
        clock_gettime(ClockId::CLOCK_MONOTONIC).map_err(|_| ClockUnavailable::ClockUnavailable)?;
    let nanos = i128::from(at.tv_sec()) * 1_000_000_000 + i128::from(at.tv_nsec());
    i64::try_from(nanos).map_err(|_| ClockUnavailable::ClockRange)
}

#[cfg(target_os = "linux")]
fn monotonic_offset_ns(contents: &str) -> Result<i64, ClockUnavailable> {
    let mut monotonic = None;
    for line in contents.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("monotonic") {
            continue;
        }
        if monotonic.is_some() {
            return Err(ClockUnavailable::DomainUnavailable);
        }
        let seconds = fields.next().and_then(|value| value.parse::<i64>().ok());
        let nanos = fields.next().and_then(|value| value.parse::<u32>().ok());
        let (Some(seconds), Some(nanos), None) = (seconds, nanos, fields.next()) else {
            return Err(ClockUnavailable::DomainUnavailable);
        };
        if nanos >= 1_000_000_000 {
            return Err(ClockUnavailable::DomainUnavailable);
        }
        monotonic = seconds
            .checked_mul(1_000_000_000)
            .and_then(|seconds| seconds.checked_add(i64::from(nanos)));
        if monotonic.is_none() {
            return Err(ClockUnavailable::ClockRange);
        }
    }
    monotonic.ok_or(ClockUnavailable::DomainUnavailable)
}

#[cfg(all(test, target_os = "linux"))]
#[path = "native_clock_tests.rs"]
mod tests;
