use crate::{Event, decode_event_entry};
use xai_tool_protocol::tool_execution::{ToolCallOwnership, ToolExecutionTiming};

#[test]
fn canonical_pure_decoder_retains_the_same_start_point_and_original_owner() {
    let started = decode_event_entry(r#"{"ts":"2026-10-28T00:00:00.000Z","type":"tool_execution","tool_call_id":"reused-provider","owner":{"ownership":"registered","invocationId":"original-invocation","providerCallId":"reused-provider","source":{"source":"sampling","promptId":"original-prompt","requestId":"original-request","attempt":2}},"timing":{"state":"started","executionId":"12345678-1234-4234-8234-123456789abc","startedAt":{"clock":"linux_monotonic","domain":{"boot_id":"b40c3d02-9538-44a4-835f-4e4bb164d8cc","namespace_device":4,"namespace_inode":4026531834,"monotonic_offset_ns":0},"earliest_ns":100,"latest_ns":110}}}"#).unwrap();
    let finished = decode_event_entry(r#"{"ts":"2040-01-01T00:00:00.000Z","type":"tool_execution","tool_call_id":"reused-provider","owner":{"ownership":"registered","invocationId":"original-invocation","providerCallId":"reused-provider","source":{"source":"sampling","promptId":"original-prompt","requestId":"original-request","attempt":2}},"timing":{"state":"finished","executionId":"12345678-1234-4234-8234-123456789abc","startedAt":{"clock":"linux_monotonic","domain":{"boot_id":"b40c3d02-9538-44a4-835f-4e4bb164d8cc","namespace_device":4,"namespace_inode":4026531834,"monotonic_offset_ns":0},"earliest_ns":100,"latest_ns":110},"durationMs":2000,"termination":{"kind":"returned"}}}"#).unwrap();
    let Event::ToolExecution { owner: start_owner, timing: ToolExecutionTiming::Started { execution_id: first, started_at: start }, .. } = started.event else {
        panic!("canonical Started");
    };
    let Event::ToolExecution { owner: finish_owner, timing: ToolExecutionTiming::Finished { execution_id: last, started_at: end, duration_ms: 2000, .. }, .. } = finished.event else {
        panic!("canonical Finished");
    };
    assert_eq!(first, last);
    assert_eq!(start, end);
    assert_eq!(start_owner, finish_owner);
    assert!(matches!(start_owner, ToolCallOwnership::Registered { .. }));
}

#[test]
fn canonical_pure_decoder_distinguishes_missing_legacy_point_from_malformed_provided_point() {
    let legacy = decode_event_entry(r#"{"ts":"2026-10-28T00:00:00.000Z","type":"tool_execution","tool_call_id":"legacy","timing":{"state":"started","executionId":"12345678-1234-4234-8234-123456789abc"}}"#).unwrap();
    let Event::ToolExecution { owner: ToolCallOwnership::Unknown, timing: ToolExecutionTiming::Started { started_at, .. }, .. } = legacy.event else {
        panic!("legacy source has no owner proof");
    };
    assert_eq!(started_at, Default::default());
    assert!(decode_event_entry(r#"{"ts":"2026-10-28T00:00:00.000Z","type":"tool_execution","tool_call_id":"broken","timing":{"state":"started","executionId":"12345678-1234-4234-8234-123456789abc","startedAt":{"clock":"linux_monotonic","domain":{"boot_id":"b40c3d02-9538-44a4-835f-4e4bb164d8cc","namespace_device":4,"namespace_inode":4026531834,"monotonic_offset_ns":0},"earliest_ns":110,"latest_ns":100}}}"#).is_err());
}
