use super::*;
use std::cell::RefCell;

fn fixture_record() -> Record {
    Record {
        boot_id: uuid::Uuid::now_v7(),
        generation: uuid::Uuid::now_v7(),
        pid_namespace_inode: std::fs::metadata("/proc/self/ns/pid").unwrap().ino(),
        process: ProcessIdentity { pid: 123, start_ticks: 456, uid: 789, proc_inode: 101 },
    }
}

fn registration_path(root: &Path, registration: &Registration) -> PathBuf {
    root.join(registration.name.to_str().unwrap())
}

fn crash_image(root: &Path, record: &Record) -> PathBuf {
    let registration = register_at(root, record).expect("durable registration");
    let path = registration_path(&root, &registration);
    let contents = std::fs::read(&path).expect("read persisted record");
    drop(registration);
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600)
        .open(&path).expect("restore crash image without a live lease");
    file.write_all(&contents).expect("restore persisted bytes");
    file.sync_all().expect("persist crash image");
    path
}

#[test]
fn live_terminal_lease_is_never_probed_or_signalled_by_startup() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let record = fixture_record();
    let registration = register_at(&root, &record).unwrap();
    let summary = sweep_at(&root, record.boot_id,
        |_, _| panic!("a live owner must never be probed"),
        |_| panic!("a live owner must never be signalled")).unwrap();
    assert_eq!(summary.examined, 1);
    assert_eq!(summary.signalled, 0);
    assert!(registration_path(&root, &registration).exists());
    let path = registration_path(&root, &registration);
    drop(registration);
    assert!(!path.exists(), "normal lifecycle removes the durable record");
}

#[test]
fn live_ownership_leases_preserve_both_current_and_other_generations() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let current = fixture_record();
    let mut other = fixture_record();
    other.boot_id = current.boot_id;
    assert_ne!(current.generation, other.generation);
    let current_lease = register_at(&root, &current).unwrap();
    let other_lease = register_at(&root, &other).unwrap();
    let summary = sweep_at(&root, current.boot_id,
        |_, _| panic!("neither live generation may be probed"),
        |_| panic!("neither live generation may be signalled")).unwrap();
    assert_eq!(summary.examined, 2);
    assert_eq!(summary.signalled, 0);
    assert!(registration_path(&root, &current_lease).exists());
    assert!(registration_path(&root, &other_lease).exists());
}

#[test]
fn legacy_pid_only_records_never_authorize_process_probing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    RegistryDirectory::create(&root).unwrap();
    let path = root.join(format!("{}.json", uuid::Uuid::now_v7()));
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).unwrap();
    file.write_all(br#"{"task_id":"legacy","pid":123,"process_group_id":123,"leader_generation":null}"#).unwrap();
    let summary = sweep_at(&root, uuid::Uuid::now_v7(),
        |_, _| panic!("legacy integers cannot authorize a probe"),
        |_| panic!("legacy integers cannot authorize a signal")).unwrap();
    assert_eq!(summary.signalled, 0);
    assert_eq!(summary.retained, 1);
    assert!(path.exists());
}

#[test]
fn empty_registry_never_probes_or_signals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    RegistryDirectory::create(&root).unwrap();
    let summary = sweep_at(&root, uuid::Uuid::now_v7(),
        |_, _| panic!("no records means no process facts"),
        |_| panic!("no records means no signals")).unwrap();
    assert_eq!(summary.examined, 0);
    assert_eq!(summary.coverage, SweepCoverage::Complete);
    assert_eq!(summary.signalled, 0);
}

#[test]
fn restart_preserves_unverifiable_groups_instead_of_guessing_a_pid() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let record = fixture_record();
    let path = crash_image(&root, &record);
    let summary = sweep_at(&root, record.boot_id,
        |_, _| Ok(Candidate::IdentityUnavailable),
        |_| panic!("missing/reused group leader cannot authorize a signal")).unwrap();
    assert_eq!(summary.retained, 1);
    assert_eq!(summary.signalled, 0);
    assert!(path.exists());
}

#[test]
fn unsupported_group_pidfds_have_no_integer_signal_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let record = fixture_record();
    let path = crash_image(&root, &record);
    let summary = sweep_at(&root, record.boot_id,
        |_, _| Err(io::Error::from_raw_os_error(libc::EINVAL)),
        |_| panic!("unsupported kernel must not fall back to killpg")).unwrap();
    assert_eq!(summary.retained, 1);
    assert!(path.exists());
}

#[test]
fn stale_boot_cannot_target_a_process_in_the_current_boot() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let record = fixture_record();
    let path = crash_image(&root, &record);
    let summary = sweep_at(&root, uuid::Uuid::now_v7(), probe,
        |_| panic!("a different boot never authorizes a signal")).unwrap();
    assert_eq!(summary.signalled, 0);
    assert!(!path.exists());
}

#[test]
fn another_pid_namespace_never_authorizes_a_signal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let mut record = fixture_record();
    record.boot_id = boot_id().unwrap();
    record.pid_namespace_inode = 0;
    let path = crash_image(&root, &record);
    let summary = sweep_at(&root, record.boot_id, probe,
        |_| panic!("another namespace cannot authorize a signal")).unwrap();
    assert_eq!(summary.signalled, 0);
    assert_eq!(summary.retained, 1);
    assert!(path.exists());
}

#[test]
fn corrupt_and_symlink_records_never_reach_process_probing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    RegistryDirectory::create(&root).unwrap();
    let outside = temp.path().join("outside");
    std::fs::write(&outside, b"untouched").unwrap();
    std::os::unix::fs::symlink(&outside, root.join(format!("{}.json", uuid::Uuid::now_v7()))).unwrap();
    let mut corrupt = OpenOptions::new().write(true).create_new(true).mode(0o600)
        .open(root.join(format!("{}.json", uuid::Uuid::now_v7()))).unwrap();
    corrupt.write_all(b"{").unwrap();
    let summary = sweep_at(&root, uuid::Uuid::now_v7(),
        |_, _| panic!("invalid records cannot authorize probes"),
        |_| panic!("invalid records cannot authorize signals")).unwrap();
    assert_eq!(summary.retained, 2);
    assert_eq!(std::fs::read(outside).unwrap(), b"untouched");
}

#[test]
fn startup_work_is_capped_even_for_unregistered_directory_entries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    RegistryDirectory::create(&root).unwrap();
    for index in 0..MAX_RECORDS + 3 {
        std::fs::write(root.join(format!("{index}.pending")), b"").unwrap();
    }
    let summary = sweep_at(&root, uuid::Uuid::now_v7(),
        |_, _| panic!("pending files are not registrations"),
        |_| panic!("pending files never authorize signals")).unwrap();
    assert_eq!(summary.examined, MAX_RECORDS);
    assert_eq!(summary.coverage, SweepCoverage::Incomplete { limit: MAX_RECORDS });
    assert_eq!(summary.signalled, 0);
}

fn descriptor_token() -> OwnedFd {
    File::open("/dev/null").unwrap().into()
}

#[test]
fn syscall_boundary_uses_the_pinned_descriptor_and_group_flag_without_a_pid_fallback() {
    let token = descriptor_token();
    signal_group_with(&token, libc::SIGKILL, |fd, signal, info, flags| {
        assert_eq!(fd, token.as_raw_fd());
        assert_eq!(signal, libc::SIGKILL);
        assert!(info.is_null());
        assert_eq!(flags, 4, "Linux PIDFD_SIGNAL_PROCESS_GROUP ABI");
        Ok(())
    }).unwrap();
    let error = signal_group_with(&token, libc::SIGKILL, |_, _, _, _| {
        Err(io::Error::from_raw_os_error(libc::EINVAL))
    }).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::EINVAL));
}

#[test]
fn verified_startup_identity_reaches_only_the_injected_signal_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let record = fixture_record();
    let path = crash_image(&root, &record);
    let events = RefCell::new(Vec::new());
    let expected = record.process.clone();
    let summary = sweep_at(&root, record.boot_id, |record, boot| {
        probe_with(record, boot, record.pid_namespace_inode,
            |pid| { assert_eq!(pid, expected.pid); events.borrow_mut().push("open"); Ok(descriptor_token()) },
            |pid| { assert_eq!(pid, expected.pid); events.borrow_mut().push("identity"); Ok(expected.clone()) },
            |_| { events.borrow_mut().push("capability"); Ok(()) })
    }, |fd| {
        signal_group_with(fd, libc::SIGKILL, |_, signal, _, flags| {
            assert_eq!((signal, flags), (libc::SIGKILL, 4));
            events.borrow_mut().push("signal");
            Ok(())
        })
    }).unwrap();
    assert_eq!(*events.borrow(), vec!["open", "identity", "capability", "signal"]);
    assert_eq!(summary.signalled, 1);
    assert_eq!(summary.retained, 0);
    assert!(!path.exists(), "accepted signal retires the record, not a claim of reaping");
}

#[test]
fn duplicate_owned_identity_is_signalled_once_while_distinct_groups_remain_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let record = fixture_record();
    let mut second = fixture_record();
    second.boot_id = record.boot_id;
    second.process.pid += 1;
    let first_path = crash_image(&root, &record);
    let duplicate_path = crash_image(&root, &record);
    let second_path = crash_image(&root, &second);
    let probed = RefCell::new(Vec::new());
    let calls = RefCell::new(Vec::new());
    let summary = sweep_at(&root, record.boot_id, |record, boot| {
        probed.borrow_mut().push(record.process.pid);
        probe_with(record, boot, record.pid_namespace_inode,
            |_| Ok(descriptor_token()), |_| Ok(record.process.clone()),
            |fd| signal_group_with(fd, 0, |_, signal, info, flags| {
                assert_eq!((signal, flags), (0, 4));
                assert!(info.is_null());
                Ok(())
            }))
    }, |fd| signal_group_with(fd, libc::SIGKILL, |raw, signal, info, flags| {
        assert_eq!(raw, fd.as_raw_fd());
        assert_eq!((signal, flags), (libc::SIGKILL, 4));
        assert!(info.is_null());
        calls.borrow_mut().push(raw);
        Ok(())
    })).unwrap();
    let mut pids = probed.into_inner();
    pids.sort_unstable();
    assert_eq!(pids, vec![record.process.pid, second.process.pid]);
    assert_eq!(calls.borrow().len(), 2);
    assert_eq!(summary.signalled, 2);
    assert_eq!(summary.examined, 3);
    assert!(!first_path.exists());
    assert!(!duplicate_path.exists());
    assert!(!second_path.exists());
}

#[test]
fn each_persisted_identity_mismatch_blocks_the_capability_and_signal_boundaries() {
    let record = fixture_record();
    let mut identities = vec![record.process.clone(); 4];
    identities[0].pid += 1;
    identities[1].start_ticks += 1;
    identities[2].uid += 1;
    identities[3].proc_inode += 1;
    for identity in identities {
        let candidate = probe_with(&record, record.boot_id, record.pid_namespace_inode,
            |_| Ok(descriptor_token()), |_| Ok(identity),
            |_| panic!("identity mismatch must not reach capability probing")).unwrap();
        assert!(matches!(candidate, Candidate::IdentityUnavailable));
    }
}

#[test]
fn group_leader_disappearing_after_pidfd_open_never_reaches_the_signal_boundary() {
    let record = fixture_record();
    let candidate = probe_with(&record, record.boot_id, record.pid_namespace_inode,
        |_| Ok(descriptor_token()),
        |_| Err(io::Error::new(io::ErrorKind::NotFound, "leader vanished")),
        |_| panic!("vanished leader cannot authorize a signal")).unwrap();
    assert!(matches!(candidate, Candidate::IdentityUnavailable));
}

#[test]
fn missing_group_leader_before_pidfd_open_is_unverifiable_not_already_gone() {
    let record = fixture_record();
    let candidate = probe_with(&record, record.boot_id, record.pid_namespace_inode,
        |_| Err(io::Error::from_raw_os_error(libc::ESRCH)),
        |_| panic!("an absent group leader cannot establish group liveness"),
        |_| panic!("an absent group leader cannot authorize a signal")).unwrap();
    assert!(matches!(candidate, Candidate::IdentityUnavailable));
}

#[test]
fn signal_failure_after_capability_admission_retains_the_record_without_retry() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let record = fixture_record();
    let path = crash_image(&root, &record);
    let mut calls = 0;
    let summary = sweep_at(&root, record.boot_id,
        |record, boot| probe_with(record, boot, record.pid_namespace_inode,
            |_| Ok(descriptor_token()), |_| Ok(record.process.clone()), |_| Ok(())),
        |fd| signal_group_with(fd, libc::SIGKILL, |_, _, _, _| {
            calls += 1;
            Err(io::Error::from_raw_os_error(libc::ESRCH))
        })).unwrap();
    assert_eq!(calls, 1);
    assert_eq!(summary.signalled, 0);
    assert_eq!(summary.retained, 1);
    assert!(path.exists());
}

#[test]
fn unit_test_runtime_disallows_destructive_kernel_signals() {
    let error = signal_group(&descriptor_token(), libc::SIGKILL).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
}

#[tokio::test]
async fn terminal_caller_waits_for_the_durable_writer_before_publishing_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let record = fixture_record();
    let expected_pid = record.process.pid;
    let registry = root.clone();
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let caller = tokio::spawn(super::super::write_spawned_process_registration(Some(expected_pid), move |pid| {
        assert_eq!(pid, expected_pid);
        entered_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        register_at(&registry, &record).map(Some)
    }));
    entered_rx.recv().await.unwrap();
    assert!(!caller.is_finished(), "caller must not publish before the writer completes");
    release_tx.send(()).unwrap();
    let registration = caller.await.unwrap().unwrap().unwrap();
    let bytes = std::fs::read(registration_path(&root, &registration)).unwrap();
    let persisted: Record = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(persisted.process.pid, expected_pid);
    let summary = sweep_at(&root, persisted.boot_id,
        |_, _| panic!("published caller must retain the live ownership lease"),
        |_| panic!("live caller cannot be signalled")).unwrap();
    assert_eq!(summary.signalled, 0);
    let path = registration_path(&root, &registration);
    drop(registration);
    assert!(!path.exists());
}

#[test]
fn mismatched_procfs_pid_coordinates_fail_closed_before_numeric_process_use() {
    fn view() -> ProcViewIdentity {
        ProcViewIdentity {
            caller_pid: 42,
            reported_pid: 42,
            self_directory: FileIdentity { device: 1, inode: 2 },
            numeric_directory: FileIdentity { device: 1, inode: 2 },
            self_namespace: FileIdentity { device: 3, inode: 4 },
            init_namespace: FileIdentity { device: 3, inode: 4 },
        }
    }
    admit_proc_view(view()).unwrap();
    let mut wrong_reported_pid = view();
    wrong_reported_pid.reported_pid += 1;
    assert_eq!(admit_proc_view(wrong_reported_pid).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    let mut wrong_numeric_process = view();
    wrong_numeric_process.numeric_directory.inode += 1;
    assert_eq!(admit_proc_view(wrong_numeric_process).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    let mut wrong_mounted_namespace = view();
    wrong_mounted_namespace.init_namespace.inode += 1;
    assert_eq!(admit_proc_view(wrong_mounted_namespace).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn all_record_operations_remain_on_the_verified_directory_after_path_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let original = temp.path().join("original");
    let outside = temp.path().join("outside");
    let record = fixture_record();
    let path = crash_image(&root, &record);
    let name = path.file_name().unwrap().to_owned();
    let directory = RegistryDirectory::open(&root).unwrap();
    std::fs::rename(&root, &original).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let outside_mode = std::fs::metadata(&outside).unwrap().mode();
    std::fs::write(outside.join(&name), b"outside must remain untouched").unwrap();
    std::os::unix::fs::symlink(&outside, &root).unwrap();
    assert!(RegistryDirectory::open(&root).is_err(), "replacement symlink must never be followed");
    assert!(RegistryDirectory::create(&root).is_err(), "existing symlink must never be chmodded or followed");
    assert_eq!(std::fs::metadata(&outside).unwrap().mode(), outside_mode);
    let summary = sweep_in(Arc::clone(&directory), record.boot_id,
        |loaded, _| { assert_eq!(loaded.process, record.process); Ok(Candidate::ForeignBoot) },
        |_| panic!("obsolete records do not authorize signals")).unwrap();
    assert_eq!(summary.examined, 1);
    assert!(!original.join(&name).exists());
    assert_eq!(std::fs::read(outside.join(&name)).unwrap(), b"outside must remain untouched");
    let registration = register_in(directory, &record).unwrap();
    let new_name = registration.name.to_str().unwrap().to_owned();
    assert!(original.join(&new_name).exists());
    assert!(!outside.join(&new_name).exists());
    drop(registration);
    assert!(!original.join(&new_name).exists(), "retirement must also use the held directory fd");
}

#[tokio::test]
async fn existing_process_scope_remains_reachable_during_a_slow_durable_writer() {
    let scope = crate::util::ProcessScope::new();
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    // An unattached group exercises production ordering without naming a real process.
    let caller = super::super::enroll_spawned_with_registration(
        &scope, None, crate::util::ProcessGroup::new().unwrap(), Some(123), move |pid| {
            assert_eq!(pid, 123);
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(Some(()))
        },
    );
    tokio::pin!(caller);
    tokio::select! {
        _ = &mut caller => panic!("production enrollment must not finish before persistence"),
        entered = tokio::time::timeout(std::time::Duration::from_secs(3), entered_rx.recv()) => {
            entered.unwrap().unwrap();
        }
    }
    assert_eq!(scope.live_count(), 1, "production must enroll before entering the blocked writer");
    release_tx.send(()).unwrap();
    let (group, registration) = caller.await;
    assert_eq!(registration, Some(()));
    assert_eq!(Arc::strong_count(&group), 1, "production must retain the same sole strong registration");
    assert_eq!(scope.live_count(), 1);
    drop(group);
    assert_eq!(scope.live_count(), 0, "releasing production's strong handle must retire that scope entry");
}

#[tokio::test]
async fn terminal_caller_with_no_child_never_invokes_the_writer() {
    let registration = super::super::write_spawned_process_registration::<()>(None,
        |_| panic!("no child cannot create an ownership row")).await.unwrap();
    assert!(registration.is_none());
}

#[tokio::test]
async fn terminal_caller_preserves_writer_failures_without_fabricating_ownership() {
    let error = super::super::write_spawned_process_registration::<()>(Some(123),
        |_| Err(io::Error::from_raw_os_error(libc::ENOSPC))).await.unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ENOSPC));
}
