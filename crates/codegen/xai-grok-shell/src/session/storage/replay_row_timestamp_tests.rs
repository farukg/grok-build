use super::*;
use crate::extensions::tool_timing::{
    ToolExecutionId, ToolExecutionTermination, ToolExecutionTiming, timing_from_meta,
};
use serde_json::json;

#[test]
fn persisted_collapse_preserves_registration_terminal_metadata_and_execution_duration() {
    let execution = ToolExecutionId::generate();
    let terminal = ToolExecutionTiming::Finished {
        execution_id: execution,
        started_at: Default::default(),
        duration_ms: 42_000,
        termination: ToolExecutionTermination::Returned,
    };
    let lines = [
        acp_envelope_with_meta(
            r#"{"sessionUpdate":"tool_call","toolCallId":"late","title":"write","status":"pending"}"#,
            r#"{"agentTimestampMs":1793367966934,"eventId":"s-1","promptId":"original"}"#,
        ),
        acp_envelope_with_meta(
            r#"{"sessionUpdate":"tool_call_update","toolCallId":"late","title":"renamed"}"#,
            r#"{"agentTimestampMs":1793370000000,"eventId":"s-2"}"#,
        ),
        acp_envelope_with_meta(
            r#"{"sessionUpdate":"tool_call_update","toolCallId":"late","status":"pending","_meta":{"progressMarker":"retained","toolRegistrationTimestampMs":1793380000000}}"#,
            r#"{"agentTimestampMs":1793380000000,"eventId":"s-progress"}"#,
        ),
        acp_envelope_with_meta(
            &json!({"sessionUpdate":"tool_call_update","toolCallId":"late","status":"completed",
                "_meta":{"toolExecutionTiming":terminal}}).to_string(),
            r#"{"agentTimestampMs":1793390000000,"eventId":"s-3","promptId":"original","future":{"retained":true}}"#,
        ),
        acp_envelope_with_meta(
            r#"{"sessionUpdate":"tool_call","toolCallId":"eof","title":"unfinished"}"#,
            r#"{"agentTimestampMs":1793368000000,"eventId":"s-4"}"#,
        ),
        acp_envelope_with_meta(
            r#"{"sessionUpdate":"tool_call","toolCallId":"unknown","title":"legacy","status":"pending","_meta":{"toolRegistrationTimestampMs":null}}"#,
            r#"{"agentTimestampMs":1793390000000}"#,
        ),
        acp_envelope_with_meta(
            r#"{"sessionUpdate":"tool_call_update","toolCallId":"unknown","status":"pending","_meta":{"toolRegistrationTimestampMs":1793390000001,"progressMarker":"legacy retained"}}"#,
            r#"{"agentTimestampMs":1793390000001}"#,
        ),
    ];
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("sessions/cwd/child");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(SUMMARY_FILE), "{}").unwrap();
    std::fs::write(dir.join(UPDATES_FILE), lines.join("\n")).unwrap();
    let mut updates = Vec::new();
    stream_replay_updates_at_hinted("child", home.path(), ReplayPathHint::default(),
        |update| updates.push(update)).unwrap();
    assert_eq!(updates.len(), 3);
    let ReplayedUpdate::Acp(acp::SessionUpdate::ToolCall(call), meta) = &updates[0] else {
        panic!("collapsed tool required");
    };
    assert_eq!(call.title, "renamed");
    assert_eq!(call.meta.as_ref().unwrap().get("toolRegistrationTimestampMs"),
        Some(&json!(1793367966934_i64)));
    assert_eq!(call.meta.as_ref().unwrap().get("progressMarker"),
        Some(&json!("retained")));
    assert_eq!(meta.as_ref().unwrap(), json!({
        "agentTimestampMs":1793390000000_i64,"eventId":"s-3","promptId":"original",
        "future":{"retained":true}
    }).as_object().unwrap());
    assert!(matches!(timing_from_meta(call.meta.as_ref()).unwrap(),
        ToolExecutionTiming::Finished { execution_id, duration_ms: 42_000,
            termination: ToolExecutionTermination::Returned, .. } if execution_id == execution));
    let ReplayedUpdate::Acp(acp::SessionUpdate::ToolCall(call), meta) = &updates[1] else {
        panic!("EOF tool required");
    };
    assert!(meta.is_none(), "EOF must not invent terminal metadata");
    assert_eq!(call.meta.as_ref().unwrap().get("toolRegistrationTimestampMs"),
        Some(&json!(1793368000000_i64)));
    let ReplayedUpdate::Acp(acp::SessionUpdate::ToolCall(call), meta) = &updates[2] else {
        panic!("unknown-origin EOF tool required");
    };
    assert!(meta.is_none());
    assert_eq!(call.meta.as_ref().unwrap().get("toolRegistrationTimestampMs"), Some(&json!(null)));
    assert_eq!(call.meta.as_ref().unwrap().get("progressMarker"),
        Some(&json!("legacy retained")));
}
