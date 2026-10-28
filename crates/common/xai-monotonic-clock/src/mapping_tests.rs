use super::*;
use crate::LinuxClockDomain;

fn domain() -> LinuxClockDomain {
    serde_json::from_str(
        r#"{
        "boot_id":"b40c3d02-9538-44a4-835f-4e4bb164d8cc",
        "namespace_device":4,"namespace_inode":4026531834,"monotonic_offset_ns":0
    }"#,
    )
    .unwrap()
}

fn sample(local: Instant, before_ns: i64, after_ns: i64) -> ClockSample {
    ClockSample {
        domain: domain(),
        local: local.into_std(),
        before_ns,
        after_ns,
    }
}

fn bounds(mapping: LocalClockMapping) -> (Instant, Instant) {
    let LocalClockMapping::Correlated {
        earliest_at,
        latest_at,
    } = mapping
    else {
        panic!("expected correlated clock");
    };
    (earliest_at, latest_at)
}

#[test]
fn calibration_bounds_contain_the_source_anchor_after_delivery_delay() {
    let started = Instant::now();
    let anchor = started + Duration::from_secs(1200);
    let clock =
        MonotonicPoint::from_sample(anchor, sample(started, 100_000_000_000, 100_000_000_010))
            .unwrap();
    let projection = clock;
    let received = started + Duration::from_secs(90);
    let (earliest, latest) =
        bounds(projection.map_sample(&sample(received, 190_000_000_000, 190_000_000_020)));
    assert_eq!(earliest, anchor - Duration::from_nanos(20));
    assert_eq!(
        latest,
        anchor + Duration::from_millis(1) + Duration::from_nanos(10)
    );
    assert!(earliest <= anchor && anchor <= latest);
    assert!(latest < received + Duration::from_secs(1200));
}

#[test]
fn wall_jumps_and_suspend_do_not_advance_the_awake_clock_deadline() {
    let started = Instant::now();
    let clock = MonotonicPoint::from_sample(
        started + Duration::from_secs(1200),
        sample(started, 100_000_000_000, 100_000_000_010),
    )
    .unwrap();
    let projection = clock;
    let after_thirty_awake_seconds = sample(
        started + Duration::from_secs(30),
        130_000_000_000,
        130_000_000_020,
    );
    let before_suspend = projection.map_sample(&after_thirty_awake_seconds);
    // A Linux suspend advances wall/boottime, not either CLOCK_MONOTONIC sample.
    assert_eq!(
        projection.map_sample(&after_thirty_awake_seconds),
        before_suspend
    );
    let (earliest, latest) = bounds(before_suspend);
    assert!(earliest > started + Duration::from_secs(1199));
    assert!(latest >= started + Duration::from_secs(1200));
}

#[test]
fn foreign_boot_namespace_and_offset_are_unknown() {
    let now = Instant::now();
    let clock = MonotonicPoint::from_sample(now + Duration::from_secs(1200), sample(now, 100, 110))
        .unwrap();
    let projection = clock;
    for field in [
        "boot_id",
        "namespace_device",
        "namespace_inode",
        "monotonic_offset_ns",
    ] {
        let mut receiver = sample(now, 100, 110);
        let mut wire = serde_json::to_value(&receiver.domain).unwrap();
        let replacement = match field {
            "boot_id" => serde_json::json!("f6ea3b21-211b-42ab-b2ec-aebd78ee2dd5"),
            "namespace_device" => serde_json::json!(5),
            "namespace_inode" => serde_json::json!(4026531835u64),
            "monotonic_offset_ns" => serde_json::json!(1),
            _ => unreachable!(),
        };
        *wire.get_mut(field).unwrap() = replacement;
        receiver.domain = serde_json::from_value(wire).unwrap();
        assert_eq!(
            projection.map_sample(&receiver),
            LocalClockMapping::Uncorrelated,
            "{field}"
        );
    }
}

#[test]
fn invalid_or_unrepresentable_clock_arithmetic_is_unknown_not_due() {
    let now = Instant::now();
    let overflowing = MonotonicPoint::from_sample(
        now + Duration::from_nanos(10),
        sample(now, i64::MAX - 1, i64::MAX),
    );
    assert_eq!(overflowing, Err(ClockUnavailable::ClockRange));
    for (earliest_ns, latest_ns) in [(11, 10), (i64::MIN, i64::MIN), (i64::MAX, i64::MAX)] {
        let clock = MonotonicPoint::LinuxMonotonic {
            domain: domain(),
            earliest_ns,
            latest_ns,
        };
        let projection = clock;
        let native_ns = if earliest_ns == i64::MAX { -1 } else { 1 };
        assert_eq!(
            projection.map_sample(&sample(now, native_ns, native_ns)),
            LocalClockMapping::Uncorrelated
        );
    }
}
