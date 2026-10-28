use super::*;
use xai_monotonic_clock::ClockUnavailable;

fn bounds(mapping: RetryClockMapping) -> (Instant, Instant) {
    let RetryClockMapping::Correlated {
        earliest_due_at,
        latest_due_at,
    } = mapping
    else {
        panic!("expected correlated clock");
    };
    (earliest_due_at, latest_due_at)
}

#[tokio::test(start_paused = true)]
async fn delayed_delivery_uses_the_original_twenty_minute_deadline() {
    let started = Instant::now();
    let deadline = RetryDeadline::after(Duration::from_secs(1200)).unwrap();
    tokio::time::advance(Duration::from_secs(90)).await;
    assert_eq!(deadline.remaining(), Duration::from_secs(1110));
    deadline.wait().await.unwrap();
    assert_eq!(Instant::now(), started + Duration::from_secs(1200));
}

#[cfg(target_os = "linux")]
#[tokio::test(start_paused = true)]
async fn native_projection_preserves_a_tokio_controlled_anchor() {
    let started = Instant::now();
    let deadline = RetryDeadline::after(Duration::from_secs(1200)).unwrap();
    let projection = deadline.projection();
    tokio::time::advance(Duration::from_secs(90)).await;
    let (earliest, latest) = bounds(projection.map_local());
    let anchor = started + Duration::from_secs(1200);
    assert!(earliest <= anchor && anchor <= latest);
    deadline.wait().await.unwrap();
    assert_eq!(Instant::now(), anchor);
}

#[tokio::test(start_paused = true)]
async fn controlled_clock_still_waits_when_native_projection_is_unavailable() {
    let started = Instant::now();
    let mut deadline = RetryDeadline::after(Duration::from_secs(1200)).unwrap();
    deadline.projection.clock = MonotonicPoint::Unavailable {
        reason: ClockUnavailable::DomainUnavailable,
    };
    assert_eq!(
        deadline.projection().map_local(),
        RetryClockMapping::Uncorrelated
    );
    deadline.wait().await.unwrap();
    assert_eq!(Instant::now(), started + Duration::from_secs(1200));
}

#[tokio::test(start_paused = true)]
async fn overflowing_scheduler_range_is_a_typed_construction_failure() {
    assert!(matches!(
        RetryDeadline::after(Duration::MAX),
        Err(RetryScheduleCause::InstantRange)
    ));
}

#[tokio::test(start_paused = true)]
async fn representable_instant_outside_the_actual_driver_range_is_rejected() {
    let backoff = Duration::from_millis(u64::MAX);
    assert!(Instant::now().checked_add(backoff).is_some());
    assert!(matches!(
        RetryDeadline::after(backoff),
        Err(RetryScheduleCause::Deadline(
            tokio::time::error::DeadlineError::TickOutOfRange,
        ))
    ));
}

#[cfg(target_os = "linux")]
#[tokio::test(start_paused = true)]
async fn projection_overflow_does_not_reject_a_representable_real_wait() {
    let started = Instant::now();
    let backoff = Duration::from_secs(10_000_000_000);
    let deadline = RetryDeadline::after(backoff).unwrap();
    assert_eq!(
        deadline.projection().map_local(),
        RetryClockMapping::Uncorrelated
    );
    deadline.clone().wait().await.unwrap();
    assert_eq!(Instant::now(), started + backoff);
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "subprocess fixture invoked by linux_separate_processes_share_the_scheduler_clock"]
async fn linux_projection_child() {
    use std::io::Write;

    let deadline = RetryDeadline::after(Duration::from_secs(1200)).unwrap();
    let projection = serde_json::to_string(&deadline.projection()).unwrap();
    println!("retry-clock-projection:{projection}");
    std::io::stdout().flush().unwrap();
    RetryDeadline::after(Duration::from_millis(50))
        .unwrap()
        .wait()
        .await
        .unwrap();
    println!("retry-clock-projection:{projection}");
    std::io::stdout().flush().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn linux_separate_processes_share_the_scheduler_clock() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    fn read_projection(reader: &mut impl BufRead) -> RetryDeadlineProjection {
        reader
            .lines()
            .find_map(|line| {
                let line = line.unwrap();
                line.split_once("retry-clock-projection:")
                    .map(|(_, projection)| serde_json::from_str(projection).unwrap())
            })
            .expect("child projection")
    }

    let before_spawn = Instant::now();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "retry_deadline::tests::linux_projection_child",
            "--ignored",
            "--nocapture",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let original = read_projection(&mut reader);
    let first_received = Instant::now();
    let delayed = read_projection(&mut reader);
    let mapped = delayed.map_local();
    let status = child.wait_with_output().unwrap();
    assert!(
        status.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(original, delayed);
    let (earliest, latest) = bounds(mapped);
    let uncertainty = latest.duration_since(earliest);
    assert!(earliest <= first_received + Duration::from_secs(1200));
    assert!(latest >= before_spawn + Duration::from_secs(1200));
    assert!(latest <= first_received + Duration::from_secs(1200) + uncertainty);
}

#[cfg(not(target_os = "linux"))]
#[tokio::test]
async fn unsupported_platform_has_no_precise_projection() {
    let deadline = RetryDeadline::after(Duration::from_secs(1200)).unwrap();
    assert_eq!(
        deadline.projection().clock,
        MonotonicPoint::Unavailable {
            reason: ClockUnavailable::UnsupportedPlatform
        }
    );
    assert_eq!(
        deadline.projection().map_local(),
        RetryClockMapping::Uncorrelated
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn diagnostic_wall_jumps_do_not_change_the_monotonic_deadline() {
    let deadline = RetryDeadline::after(Duration::from_secs(1200)).unwrap();
    let mut projection = deadline.projection();
    let original = projection.clock;
    let at = deadline.checked.deadline();
    for wall in [i64::MIN, i64::MAX] {
        projection.wall_deadline_ms = Some(wall);
        assert_eq!(projection.clock, original);
        let (earliest, latest) = bounds(projection.map_local());
        assert!(earliest <= at && at <= latest);
    }
}

#[test]
fn construction_without_a_runtime_preserves_the_native_failure() {
    assert!(matches!(
        RetryDeadline::after(Duration::ZERO),
        Err(RetryScheduleCause::Deadline(
            tokio::time::error::DeadlineError::MissingRuntime,
        ))
    ));
}

#[test]
fn construction_without_a_timer_driver_preserves_the_native_failure() {
    let owner = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    owner.block_on(async {
        assert!(matches!(
            RetryDeadline::after(Duration::ZERO),
            Err(RetryScheduleCause::Deadline(
                tokio::time::error::DeadlineError::TimeDisabled,
            ))
        ));
    });
}
