use super::*;

#[test]
fn pure_wire_points_round_trip_without_a_runtime_or_native_sampling() {
    let wire = r#"{"clock":"linux_monotonic","domain":{"boot_id":"b40c3d02-9538-44a4-835f-4e4bb164d8cc","namespace_device":4,"namespace_inode":4026531834,"monotonic_offset_ns":0},"earliest_ns":100,"latest_ns":110}"#;
    let point: MonotonicPoint = serde_json::from_str(wire).unwrap();
    assert_eq!(serde_json::to_value(point).unwrap(), serde_json::from_str::<serde_json::Value>(wire).unwrap());
    let unavailable: MonotonicPoint = serde_json::from_str(r#"{"clock":"unavailable","reason":"unsupported_platform"}"#).unwrap();
    assert_eq!(unavailable, MonotonicPoint::Unavailable { reason: ClockUnavailable::UnsupportedPlatform });
}

#[test]
fn provided_malformed_points_are_decode_errors_not_precise_or_unknown_defaults() {
    let valid = serde_json::json!({
        "clock": "linux_monotonic",
        "domain": { "boot_id": "b40c3d02-9538-44a4-835f-4e4bb164d8cc", "namespace_device": 4,
            "namespace_inode": 4026531834u64, "monotonic_offset_ns": 0 },
        "earliest_ns": 100, "latest_ns": 110,
    });
    let mut reversed = valid.clone();
    reversed["latest_ns"] = serde_json::json!(99);
    assert!(serde_json::from_value::<MonotonicPoint>(reversed).is_err());
    let mut invalid_domain = valid;
    invalid_domain["domain"]["boot_id"] = serde_json::json!("00000000-0000-0000-0000-000000000000");
    assert!(serde_json::from_value::<MonotonicPoint>(invalid_domain).is_err());
    assert!(serde_json::from_str::<MonotonicPoint>(r#"{"clock":"unavailable"}"#).is_err());
}
