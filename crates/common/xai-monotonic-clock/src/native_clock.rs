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
        let evidence = LinuxClockEvidence::read()?;
        // Linux std::Instant and CLOCK_MONOTONIC exclude suspend and share clock adjustments.
        let before_ns = monotonic_ns()?;
        let local = StdInstant::now();
        let after_ns = monotonic_ns()?;
        if before_ns > after_ns {
            return Err(ClockUnavailable::ClockUnavailable);
        }
        evidence.validate()?;
        Ok(Self {
            domain: evidence.domain,
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
#[derive(Clone, Copy, PartialEq, Eq)]
struct NamespaceIdentity {
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, PartialEq, Eq)]
struct NamespaceObservation {
    active_thread: NamespaceIdentity,
    thread_children: NamespaceIdentity,
    offset_owner: NamespaceIdentity,
}

#[cfg(target_os = "linux")]
impl NamespaceObservation {
    fn namespace(self) -> Result<NamespaceIdentity, ClockUnavailable> {
        if self.active_thread != self.thread_children || self.active_thread != self.offset_owner {
            return Err(ClockUnavailable::DomainUnavailable);
        }
        Ok(self.active_thread)
    }

    fn validate_against(self, later: Self) -> Result<(), ClockUnavailable> {
        self.namespace()?;
        later.namespace()?;
        if self != later {
            return Err(ClockUnavailable::DomainUnavailable);
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
struct NamespaceHandles {
    active_thread: std::fs::File,
    thread_children: std::fs::File,
    offset_owner: std::fs::File,
}

#[cfg(target_os = "linux")]
impl NamespaceHandles {
    fn read() -> Result<Self, ClockUnavailable> {
        let open = |path| std::fs::File::open(path).map_err(|_| ClockUnavailable::DomainUnavailable);
        Ok(Self {
            active_thread: open("/proc/thread-self/ns/time")?,
            thread_children: open("/proc/thread-self/ns/time_for_children")?,
            // The process-only timens_offsets entry uses the thread-group leader's children domain.
            offset_owner: open("/proc/self/ns/time_for_children")?,
        })
    }

    fn observe(&self) -> Result<NamespaceObservation, ClockUnavailable> {
        use std::os::unix::fs::MetadataExt;

        let identity = |file: &std::fs::File| -> Result<NamespaceIdentity, ClockUnavailable> {
            let metadata = file
                .metadata()
                .map_err(|_| ClockUnavailable::DomainUnavailable)?;
            Ok(NamespaceIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        };
        Ok(NamespaceObservation {
            active_thread: identity(&self.active_thread)?,
            thread_children: identity(&self.thread_children)?,
            offset_owner: identity(&self.offset_owner)?,
        })
    }
}

#[cfg(target_os = "linux")]
struct LinuxClockEvidence {
    domain: LinuxClockDomain,
    namespaces: NamespaceHandles,
}

#[cfg(target_os = "linux")]
impl LinuxClockEvidence {
    fn read() -> Result<Self, ClockUnavailable> {
        let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|_| ClockUnavailable::DomainUnavailable)?;
        let boot_id =
            Uuid::parse_str(boot_id.trim()).map_err(|_| ClockUnavailable::DomainUnavailable)?;
        if boot_id.is_nil() {
            return Err(ClockUnavailable::DomainUnavailable);
        }
        let namespaces = NamespaceHandles::read()?;
        let namespace = namespaces.observe()?.namespace()?;
        let offsets = std::fs::read_to_string("/proc/self/timens_offsets")
            .map_err(|_| ClockUnavailable::DomainUnavailable)?;
        // Offsets also distinguish namespace-inode reuse within one boot.
        let monotonic_offset_ns = monotonic_offset_ns(&offsets)?;
        Ok(Self {
            domain: LinuxClockDomain {
                boot_id,
                namespace_device: namespace.device,
                namespace_inode: namespace.inode,
                monotonic_offset_ns,
            },
            namespaces,
        })
    }

    fn validate(&self) -> Result<(), ClockUnavailable> {
        // Keep the original namespace handles open through sampling to prevent inode reuse.
        let original = self.namespaces.observe()?;
        let before = NamespaceHandles::read()?;
        original.validate_against(before.observe()?)?;
        let offsets = std::fs::read_to_string("/proc/self/timens_offsets")
            .map_err(|_| ClockUnavailable::DomainUnavailable)?;
        let after = NamespaceHandles::read()?;
        original.validate_against(after.observe()?)?;
        if monotonic_offset_ns(&offsets)? != self.domain.monotonic_offset_ns {
            return Err(ClockUnavailable::DomainUnavailable);
        }
        Ok(())
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
