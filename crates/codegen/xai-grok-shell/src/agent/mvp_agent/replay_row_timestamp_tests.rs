use super::*;
use serde_json::json;

#[tokio::test]
async fn parent_forward_preserves_registration_and_terminal_event_identity() {
    let (agent, mut rx) = build_agent_with_gateway();
    let mut collapser = crate::session::storage::ReplayToolCollapser::new();
    let start = r#"{"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"tool_call","toolCallId":"late","title":"write","status":"pending"},"_meta":{"agentTimestampMs":1793367966934,"eventId":"s-1"}}}"#;
    let title = r#"{"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"tool_call_update","toolCallId":"late","title":"renamed","status":"pending","_meta":{"pendingMarker":"retained"}},"_meta":{"agentTimestampMs":1793370000000,"eventId":"s-2"}}}"#;
    let end = r#"{"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"tool_call_update","toolCallId":"late","status":"completed","_meta":{"terminalMarker":"retained"}},"_meta":{"agentTimestampMs":1793390000000,"eventId":"s-3","future":"retained"}}}"#;
    agent.forward_raw_replay_line(start, None, None, true, &mut collapser, None);
    agent.forward_raw_replay_line(title, None, None, true, &mut collapser, None);
    assert!(rx.try_recv().is_err());
    agent.forward_raw_replay_line(end, None, None, true, &mut collapser, None);
    let AcpClientMessage::SessionNotification(notification) = rx.try_recv().unwrap() else {
        panic!("native tool notification required");
    };
    let acp::SessionUpdate::ToolCall(call) = &notification.request.update else {
        panic!("collapsed tool required");
    };
    assert_eq!(call.title, "renamed");
    assert_eq!(call.meta.as_ref().unwrap().get("toolRegistrationTimestampMs"),
        Some(&json!(1793367966934_i64)));
    assert_eq!(call.meta.as_ref().unwrap().get("pendingMarker"), Some(&json!("retained")));
    assert_eq!(call.meta.as_ref().unwrap().get("terminalMarker"), Some(&json!("retained")));
    let meta = notification.request.meta.as_ref().unwrap();
    assert_eq!(meta.get("agentTimestampMs"), Some(&json!(1793390000000_i64)));
    assert_eq!(meta.get("eventId"), Some(&json!("s-3")));
    assert_eq!(meta.get("future"), Some(&json!("retained")));
    assert_eq!(meta.get("isReplay"), Some(&json!(true)));
    notification.response_tx.send(Ok(())).unwrap();
    assert!(rx.try_recv().is_err());
}
