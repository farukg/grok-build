use super::*;

#[test]
fn standalone_protocol_retains_legacy_unknown_clock_and_independent_duration() {
    let wire = r#"{"state":"finished","executionId":"12345678-1234-4234-8234-123456789abc","durationMs":2000,"termination":{"kind":"returned"}}"#;
    let timing: ToolExecutionTiming = serde_json::from_str(wire).unwrap();
    let ToolExecutionTiming::Finished { started_at, duration_ms, termination, .. } = timing else {
        panic!("finished wire result");
    };
    assert_eq!(started_at, MonotonicPoint::default());
    assert_eq!(duration_ms, 2000);
    assert_eq!(termination, ToolExecutionTermination::Returned);
    assert_eq!(serde_json::from_value::<ToolExecutionTiming>(serde_json::to_value(timing).unwrap()).unwrap(), timing);
}

#[test]
fn malformed_provided_clock_is_not_legacy_absence() {
    let wire = r#"{"state":"started","executionId":"12345678-1234-4234-8234-123456789abc","startedAt":{"clock":"linux_monotonic","domain":{"boot_id":"b40c3d02-9538-44a4-835f-4e4bb164d8cc","namespace_device":4,"namespace_inode":4026531834,"monotonic_offset_ns":0},"earliest_ns":110,"latest_ns":100}}"#;
    assert!(serde_json::from_str::<ToolExecutionTiming>(wire).is_err());
}
