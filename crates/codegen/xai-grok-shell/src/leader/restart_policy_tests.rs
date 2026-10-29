use super::*;

fn stamp(seconds_ago: i64) -> DateTime<Utc> {
    Utc::now() - chrono::Duration::seconds(seconds_ago)
}

#[test]
fn classify_boundary_is_strictly_after_one_hour() {
    let now = Utc::now();
    assert_eq!(
        classify(now - chrono::Duration::seconds(3601), now),
        RestartStopAge::Stale
    );
    assert_eq!(
        classify(now - chrono::Duration::seconds(3600), now),
        RestartStopAge::Fresh,
        "exactly one hour is fresh"
    );
    assert_eq!(
        classify(now - chrono::Duration::seconds(3599), now),
        RestartStopAge::Fresh
    );
}

#[test]
fn classify_future_stamp_is_stale() {
    let now = Utc::now();
    assert_eq!(
        classify(now + chrono::Duration::seconds(5), now),
        RestartStopAge::Stale,
        "a clock rollback cannot prove freshness"
    );
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
        read_stop_age(&temp.path().join("absent.heartbeat"), stamp(0)),
        RecordedStopAge::FirstRun
    );
}

#[test]
fn written_heartbeat_reads_back_and_classifies() {
    let temp = tempfile::tempdir().unwrap();
    let heartbeat = heartbeat_path(&temp.path().join("leader.lock"));
    let fresh_at = stamp(600);
    write_heartbeat(&heartbeat, fresh_at).unwrap();
    assert_eq!(read_stop_age(&heartbeat, stamp(0)), RecordedStopAge::Fresh {
        recorded_at: fresh_at
    });
    let stale_at = stamp(7200);
    write_heartbeat(&heartbeat, stale_at).unwrap();
    assert_eq!(
        read_stop_age(&heartbeat, stamp(0)),
        RecordedStopAge::Stale {
            recorded_at: Some(stale_at)
        }
    );
}

#[test]
fn corrupt_heartbeat_classifies_stale() {
    let temp = tempfile::tempdir().unwrap();
    let heartbeat = heartbeat_path(&temp.path().join("leader.lock"));
    std::fs::write(&heartbeat, "not a timestamp").unwrap();
    assert_eq!(
        read_stop_age(&heartbeat, stamp(0)),
        RecordedStopAge::Stale { recorded_at: None }
    );
}
