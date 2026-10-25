//! Durable backing for terminal process ownership after an unclean host exit.
//! Signals require verified process identity and a kernel-pinned group pidfd.

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

const MAX_RECORDS: usize = 4096;
const MAX_RECORD_BYTES: u64 = 16 * 1024;
// Linux 6.9+: this targets the pidfd's struct pid, never a recycled integer PGID.
const PIDFD_SIGNAL_PROCESS_GROUP: libc::c_uint = 1 << 2;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct ProcessIdentity {
    pid: u32,
    start_ticks: u64,
    uid: u32,
    proc_inode: u64,
}

#[derive(Serialize, Deserialize)]
struct Record {
    boot_id: uuid::Uuid,
    generation: uuid::Uuid,
    pid_namespace_inode: u64,
    process: ProcessIdentity,
}

#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn read(path: &Path) -> io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self { device: metadata.dev(), inode: metadata.ino() })
    }
}

struct ProcViewIdentity {
    caller_pid: u32,
    reported_pid: u32,
    self_directory: FileIdentity,
    numeric_directory: FileIdentity,
    self_namespace: FileIdentity,
    init_namespace: FileIdentity,
}

fn admit_proc_view(view: ProcViewIdentity) -> io::Result<()> {
    if view.caller_pid != view.reported_pid
        || view.self_directory != view.numeric_directory
        || view.self_namespace != view.init_namespace
    {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "procfs does not match the caller PID namespace"));
    }
    Ok(())
}

fn verify_proc_view() -> io::Result<()> {
    // SAFETY: getpid has no preconditions.
    let caller_pid = u32::try_from(unsafe { libc::getpid() }).map_err(io::Error::other)?;
    let stat = std::fs::read_to_string("/proc/self/stat")?;
    let reported_pid = stat.split_whitespace().next().ok_or_else(invalid_stat)?
        .parse().map_err(io::Error::other)?;
    admit_proc_view(ProcViewIdentity {
        caller_pid,
        reported_pid,
        self_directory: FileIdentity::read(Path::new("/proc/self"))?,
        numeric_directory: FileIdentity::read(&PathBuf::from(format!("/proc/{caller_pid}")))?,
        self_namespace: FileIdentity::read(Path::new("/proc/self/ns/pid"))?,
        init_namespace: FileIdentity::read(Path::new("/proc/1/ns/pid"))?,
    })
}

struct RegistryDirectory {
    file: File,
}

impl RegistryDirectory {
    fn create(root: &Path) -> io::Result<Arc<Self>> {
        match std::fs::DirBuilder::new().mode(0o700).create(root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        Self::open(root)
    }

    fn open(root: &Path) -> io::Result<Arc<Self>> {
        let file = OpenOptions::new().read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(root)?;
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "unsafe ownership registry directory"));
        }
        Ok(Arc::new(Self { file }))
    }

    fn open_record(&self, name: &CStr, flags: libc::c_int) -> io::Result<File> {
        // SAFETY: name is NUL-terminated; the held directory fd anchors every operation.
        let fd = unsafe {
            libc::openat(self.file.as_raw_fd(), name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK, 0o600 as libc::mode_t)
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful openat returned a new owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn rename(&self, source: &CStr, target: &CStr) -> io::Result<()> {
        // SAFETY: both names are NUL-terminated and relative to the held directory.
        let result = unsafe {
            libc::renameat(self.file.as_raw_fd(), source.as_ptr(), self.file.as_raw_fd(), target.as_ptr())
        };
        syscall_result(result)
    }

    fn unlink(&self, name: &CStr) -> io::Result<()> {
        // SAFETY: name is NUL-terminated and relative to the held directory.
        syscall_result(unsafe { libc::unlinkat(self.file.as_raw_fd(), name.as_ptr(), 0) })
    }

    fn entries(&self) -> io::Result<(Vec<CString>, SweepCoverage)> {
        // A separate open file description avoids sharing a readdir offset with another sweep.
        let directory = self.open_record(c".", libc::O_RDONLY | libc::O_DIRECTORY)?;
        use std::os::fd::IntoRawFd;
        let fd = directory.into_raw_fd();
        // SAFETY: fd is a new owned directory descriptor. fdopendir takes ownership on success.
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: fdopendir failed, so fd is still owned here.
            drop(unsafe { File::from_raw_fd(fd) });
            return Err(error);
        }
        struct Stream(*mut libc::DIR);
        impl Drop for Stream {
            fn drop(&mut self) {
                // SAFETY: this guard uniquely owns the stream returned by fdopendir.
                unsafe { libc::closedir(self.0); }
            }
        }
        let stream = Stream(stream);
        let mut entries = Vec::new();
        loop {
            // SAFETY: Linux errno is thread-local, and stream remains uniquely owned here.
            let entry = unsafe {
                *libc::__errno_location() = 0;
                libc::readdir(stream.0)
            };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    return Err(error);
                }
                return Ok((entries, SweepCoverage::Complete));
            }
            // SAFETY: readdir returned a live dirent with a NUL-terminated d_name.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if name == c"." || name == c".." {
                continue;
            }
            if entries.len() == MAX_RECORDS {
                return Ok((entries, SweepCoverage::Incomplete { limit: MAX_RECORDS }));
            }
            entries.push(name.to_owned());
        }
    }
}

fn syscall_result(result: libc::c_int) -> io::Result<()> {
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

enum Retirement {
    Collected,
    HandedOff,
}

pub(super) struct Registration {
    directory: Arc<RegistryDirectory>,
    name: CString,
    _lease: File,
    retirement: Retirement,
}

impl Registration {
    pub(super) fn hand_off(&mut self) {
        self.retirement = Retirement::HandedOff;
    }

    pub(super) fn retire_collected(&mut self, child: &tokio::process::Child) {
        if child.id().is_none() {
            self.retirement = Retirement::Collected;
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if matches!(self.retirement, Retirement::HandedOff) {
            return;
        }
        if let Err(error) = self.directory.unlink(&self.name)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "could not retire terminal ownership record");
        }
    }
}

fn registry_root() -> PathBuf {
    crate::util::grok_home().join("owned-task-processes-v1")
}

fn boot_id() -> io::Result<uuid::Uuid> {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim().parse().map_err(io::Error::other)
}

fn process_identity(pid: u32) -> io::Result<ProcessIdentity> {
    let path = PathBuf::from(format!("/proc/{pid}"));
    let metadata = std::fs::metadata(&path)?;
    let stat = std::fs::read_to_string(path.join("stat"))?;
    let (_, fields) = stat.rsplit_once(") ").ok_or_else(invalid_stat)?;
    let fields: Vec<&str> = fields.split_whitespace().collect();
    let group: u32 = fields.get(2).ok_or_else(invalid_stat)?.parse().map_err(io::Error::other)?;
    if group != pid {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not an owned group leader"));
    }
    let start_ticks = fields.get(19).ok_or_else(invalid_stat)?.parse().map_err(io::Error::other)?;
    Ok(ProcessIdentity { pid, start_ticks, uid: metadata.uid(), proc_inode: metadata.ino() })
}

fn invalid_stat() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid process stat")
}

pub(super) fn register(pid: u32) -> io::Result<Option<Registration>> {
    register_child_at(&registry_root(), pid)
}

pub(super) fn register_child_at(root: &Path, pid: u32) -> io::Result<Option<Registration>> {
    verify_proc_view()?;
    static GENERATION: OnceLock<uuid::Uuid> = OnceLock::new();
    let process = match process_identity(pid) {
        Ok(process) => process,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let record = Record {
        boot_id: boot_id()?,
        generation: *GENERATION.get_or_init(uuid::Uuid::now_v7),
        pid_namespace_inode: std::fs::metadata("/proc/self/ns/pid")?.ino(),
        process,
    };
    let mut registration = register_at(root, &record)?;
    // A spawned child may lose its actor before installation; only collection retires its row.
    registration.hand_off();
    Ok(Some(registration))
}

fn register_at(root: &Path, record: &Record) -> io::Result<Registration> {
    register_in(RegistryDirectory::create(root)?, record)
}

fn register_in(directory: Arc<RegistryDirectory>, record: &Record) -> io::Result<Registration> {
    let id = uuid::Uuid::now_v7();
    let pending = CString::new(format!("{id}.pending")).map_err(io::Error::other)?;
    let name = CString::new(format!("{id}.json")).map_err(io::Error::other)?;
    let mut lease = directory.open_record(&pending, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL)?;
    let result = (|| {
        lease.try_lock_exclusive()?;
        serde_json::to_writer(&mut lease, record).map_err(io::Error::other)?;
        lease.flush()?;
        lease.sync_all()?;
        directory.rename(&pending, &name)?;
        directory.file.sync_all()
    })();
    if let Err(error) = result {
        let _ = directory.unlink(&pending);
        let _ = directory.unlink(&name);
        return Err(error);
    }
    Ok(Registration { directory, name, _lease: lease, retirement: Retirement::Collected })
}

fn open_pidfd(pid: u32) -> io::Result<OwnedFd> {
    // SAFETY: pidfd_open has no pointer arguments; success transfers one fd.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    let fd = i32::try_from(fd).map_err(io::Error::other)?;
    // SAFETY: successful pidfd_open returned a new owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn signal_group(fd: &OwnedFd, signal: libc::c_int) -> io::Result<()> {
    #[cfg(test)]
    if signal != 0 {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "destructive orphan signals are disabled in unit tests"));
    }
    signal_group_with(fd, signal, |fd, signal, info, flags| {
        // SAFETY: fd is owned; info is null; flags request only the pinned process group.
        let result = unsafe { libc::syscall(libc::SYS_pidfd_send_signal, fd, signal, info, flags) };
        if result < 0 { return Err(io::Error::last_os_error()); }
        Ok(())
    })
}

fn signal_group_with(
    fd: &OwnedFd,
    signal: libc::c_int,
    syscall: impl FnOnce(libc::c_int, libc::c_int, *const libc::siginfo_t, libc::c_uint) -> io::Result<()>,
) -> io::Result<()> {
    syscall(fd.as_raw_fd(), signal, std::ptr::null(), PIDFD_SIGNAL_PROCESS_GROUP)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SweepCoverage {
    #[default]
    Complete,
    Incomplete { limit: usize },
}

#[derive(Debug, Default)]
pub struct SweepSummary {
    pub examined: usize,
    pub signalled: usize,
    pub retained: usize,
    pub coverage: SweepCoverage,
}

enum Candidate {
    ForeignBoot,
    IdentityUnavailable,
    Verified(OwnedFd),
}

fn probe(record: &Record, current_boot: uuid::Uuid) -> io::Result<Candidate> {
    probe_with(record, current_boot, std::fs::metadata("/proc/self/ns/pid")?.ino(),
        open_pidfd, process_identity, |fd| signal_group(fd, 0))
}

fn probe_with(
    record: &Record,
    current_boot: uuid::Uuid,
    current_namespace: u64,
    open: impl FnOnce(u32) -> io::Result<OwnedFd>,
    identity: impl FnOnce(u32) -> io::Result<ProcessIdentity>,
    capability: impl FnOnce(&OwnedFd) -> io::Result<()>,
) -> io::Result<Candidate> {
    if record.boot_id != current_boot { return Ok(Candidate::ForeignBoot); }
    if record.pid_namespace_inode != current_namespace { return Ok(Candidate::IdentityUnavailable); }
    let fd = match open(record.process.pid) {
        Ok(fd) => fd,
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return Ok(Candidate::IdentityUnavailable),
        Err(error) => return Err(error),
    };
    match identity(record.process.pid) {
        Ok(identity) if identity == record.process => {
            // No fallback on kernels lacking pinned process-group signalling.
            capability(&fd)?;
            Ok(Candidate::Verified(fd))
        }
        Ok(_) => Ok(Candidate::IdentityUnavailable),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Candidate::IdentityUnavailable),
        Err(error) => Err(error),
    }
}

pub fn sweep() -> io::Result<SweepSummary> {
    verify_proc_view()?;
    sweep_at(&registry_root(), boot_id()?, probe, |fd| signal_group(fd, libc::SIGKILL))
}

fn sweep_at(
    root: &Path,
    current_boot: uuid::Uuid,
    probe: impl FnMut(&Record, uuid::Uuid) -> io::Result<Candidate>,
    kill: impl FnMut(&OwnedFd) -> io::Result<()>,
) -> io::Result<SweepSummary> {
    let directory = match RegistryDirectory::open(root) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(SweepSummary::default()),
        Err(error) => return Err(error),
    };
    sweep_in(directory, current_boot, probe, kill)
}

fn sweep_in(
    directory: Arc<RegistryDirectory>,
    current_boot: uuid::Uuid,
    mut probe: impl FnMut(&Record, uuid::Uuid) -> io::Result<Candidate>,
    mut kill: impl FnMut(&OwnedFd) -> io::Result<()>,
) -> io::Result<SweepSummary> {
    let (entries, coverage) = directory.entries()?;
    let mut summary = SweepSummary { coverage, ..SweepSummary::default() };
    let mut signalled_identities = HashSet::new();
    for name in entries {
        summary.examined += 1;
        let recognized = name.to_str().ok().and_then(|name| name.strip_suffix(".json"))
            .and_then(|stem| stem.parse::<uuid::Uuid>().ok());
        if recognized.is_none() { continue; }
        let result = (|| -> io::Result<()> {
            let file = directory.open_record(&name, libc::O_RDWR)?;
            let metadata = file.metadata()?;
            // SAFETY: geteuid has no preconditions.
            if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 || metadata.len() > MAX_RECORD_BYTES
            {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "unsafe ownership record"));
            }
            match file.try_lock_exclusive() {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            }
            let record: Record = serde_json::from_reader((&file).take(MAX_RECORD_BYTES)).map_err(io::Error::other)?;
            let identity = (record.boot_id, record.pid_namespace_inode, record.process.clone());
            if signalled_identities.contains(&identity) {
                directory.unlink(&name)?;
                return Ok(());
            }
            match probe(&record, current_boot)? {
                Candidate::ForeignBoot => directory.unlink(&name)?,
                Candidate::IdentityUnavailable => summary.retained += 1,
                Candidate::Verified(fd) => {
                    kill(&fd)?;
                    signalled_identities.insert(identity);
                    summary.signalled += 1;
                    // Signal acceptance is not wait/reap completion for another process's children.
                    directory.unlink(&name)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            summary.retained += 1;
            tracing::warn!(%error, "terminal orphan record retained without an unsafe fallback");
        }
    }
    match summary.coverage {
        SweepCoverage::Complete => {}
        SweepCoverage::Incomplete { limit } => {
            tracing::warn!(limit, "terminal orphan sweep incomplete; retained entries can starve later records; no automatic retry");
        }
    }
    Ok(summary)
}

#[cfg(test)]
#[path = "terminal_orphan_registry_tests.rs"]
mod tests;
