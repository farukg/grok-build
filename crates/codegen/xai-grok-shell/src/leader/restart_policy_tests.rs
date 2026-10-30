use super::*;

#[test]
fn classify_boundary_is_strictly_after_one_hour() {
    let now = Utc::now();
    for (age, expected) in [
        (chrono::Duration::hours(1) - chrono::Duration::nanoseconds(1), RestartStopAge::Fresh),
        (chrono::Duration::hours(1), RestartStopAge::Fresh),
        (chrono::Duration::hours(1) + chrono::Duration::nanoseconds(1), RestartStopAge::Stale),
    ] {
        assert_eq!(classify(now - age, now), expected);
    }
}

#[test]
fn heartbeat_path_pairs_with_the_lock_file() {
    assert_eq!(
        heartbeat_path(Path::new("/x/leader-a1b2c3d4.lock")),
        PathBuf::from("/x/leader-a1b2c3d4.heartbeat")
    );
}

#[test]
fn missing_heartbeat_is_a_first_run() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        read_stop_age(&temp.path().join("absent.heartbeat"), Utc::now()),
        RecordedStopAge::FirstRun
    );
}

#[test]
fn orderly_stop_roundtrip_obeys_exact_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let path = heartbeat_path(&temp.path().join("leader.lock"));
    let stopped_at = Utc::now();
    write_heartbeat(&path, stopped_at).unwrap();
    assert_eq!(
        read_stop_age(&path, stopped_at + chrono::Duration::hours(1)),
        RecordedStopAge::Fresh { recorded_at: stopped_at }
    );
    assert_eq!(
        read_stop_age(&path, stopped_at + chrono::Duration::hours(1) + chrono::Duration::nanoseconds(1)),
        RecordedStopAge::Stale { recorded_at: stopped_at }
    );
}

#[test]
fn running_marker_invalidates_old_stop_even_after_long_crash_gap() {
    let temp = tempfile::tempdir().unwrap();
    let path = heartbeat_path(&temp.path().join("leader.lock"));
    let stopped_at = Utc::now();
    write_heartbeat(&path, stopped_at).unwrap();
    write_running(&path).unwrap();
    let observed = read_stop_age(&path, stopped_at + chrono::Duration::days(2));
    assert_eq!(observed, RecordedStopAge::Unknown);
    assert_eq!(observed.stop_age(), RestartStopAge::Fresh);
}

#[test]
fn corrupt_legacy_and_future_evidence_never_authorize_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let path = heartbeat_path(&temp.path().join("leader.lock"));
    let now = Utc::now();
    for contents in [
        "not a timestamp".to_owned(),
        (now - chrono::Duration::days(2)).to_rfc3339(),
        format!("stopped {}", (now + chrono::Duration::seconds(1)).to_rfc3339()),
    ] {
        std::fs::write(&path, contents).unwrap();
        let observed = read_stop_age(&path, now);
        assert_eq!(observed, RecordedStopAge::Unknown);
        assert_eq!(observed.stop_age(), RestartStopAge::Fresh);
    }
}

#[test]
fn unreadable_stop_evidence_never_authorizes_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let observed = read_stop_age(temp.path(), Utc::now());
    assert_eq!(observed, RecordedStopAge::Unknown);
    assert_eq!(observed.stop_age(), RestartStopAge::Fresh);
}

#[test]
fn known_cohort_survives_unknown_crash_and_subsequent_short_stop() {
    use xai_grok_tools::types::restart_recovery::LeaderRestartRecovery;
    let directory = tempfile::tempdir().unwrap();
    let path = heartbeat_path(&directory.path().join("leader.lock"));
    let started = Utc::now();
    let cohort = LeaderRestartRecovery::stale(started).startup();
    write_running_with_startup(&path, cohort).unwrap();
    let crashed = read_evidence(&path, started + chrono::Duration::days(2)).unwrap();
    assert_eq!(crashed.stop, RecordedStopAge::Unknown);
    assert_eq!(crashed.startup, cohort);
    let recovery = LeaderRestartRecovery::from_startup(crashed.startup);
    assert!(!recovery.register("old-session", started - chrono::Duration::hours(2), Default::default()).admission().permits_recovery());
    write_stopped_with_startup(&path, started, cohort).unwrap();
    let short_stop = read_evidence(&path, started + chrono::Duration::minutes(1)).unwrap();
    assert_eq!(short_stop.stop.stop_age(), RestartStopAge::Fresh);
    assert_eq!(short_stop.startup, cohort);
}

#[test]
fn consumed_new_session_epoch_survives_clock_rollback_and_new_process() {
    use xai_grok_tools::types::restart_recovery::LeaderRestartRecovery;
    let started = Utc::now();
    let original = LeaderRestartRecovery::stale(started);
    let admitted = original.session("created-after-startup").admission();
    let encoded = serde_json::to_vec(&admitted).unwrap();
    let persisted = serde_json::from_slice(&encoded).unwrap();
    let rebuilt = LeaderRestartRecovery::from_startup(original.startup());
    assert!(rebuilt.register("created-after-startup", started - chrono::Duration::days(1), persisted).admission().permits_recovery());
}

#[test]
fn corrupt_persisted_cohort_fails_startup_closed() {
    let directory = tempfile::tempdir().unwrap();
    let path = heartbeat_path(&directory.path().join("leader.lock"));
    std::fs::write(&path, "running\nrestart not-json\n").unwrap();
    assert!(read_evidence(&path, Utc::now()).is_err());
}
