use super::*;
use crate::implementations::grok_build::task::backend::ChannelBackend;
use crate::implementations::grok_build::task::types::SubagentEvent;
use crate::types::resources::Resources;
use crate::types::tool_metadata::test_ctx;

#[tokio::test]
async fn list_subagents_closed_channel_is_an_error() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    drop(rx);
    let mut resources = Resources::new();
    resources.insert(ChannelBackend::for_session(tx, "root").into_resource());
    let error = xai_tool_runtime::Tool::run(
        &ListSubagentsTool,
        test_ctx(resources.into_shared()),
        ListSubagentsInput {},
    )
    .await
    .expect_err("transport failure must not return an empty inventory");
    assert!(error.to_string().contains("coordinator unavailable"));
    assert!(error.to_string().contains("not confirmed"));
}

#[tokio::test]
async fn list_subagents_lost_ack_is_an_error() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut resources = Resources::new();
    resources.insert(ChannelBackend::for_session(tx, "root").into_resource());
    let coordinator = async move {
        let Some(SubagentEvent::ListOwned(request)) = rx.recv().await else {
            panic!("expected a list-owned request");
        };
        assert_eq!(request.parent_session_id.as_deref(), Some("root"));
        drop(request.respond_to);
    };
    let run = xai_tool_runtime::Tool::run(
        &ListSubagentsTool,
        test_ctx(resources.into_shared()),
        ListSubagentsInput {},
    );
    let (result, ()) = tokio::join!(run, coordinator);
    let error = result.expect_err("lost response must not return an empty inventory");
    assert!(error.to_string().contains("coordinator unavailable"));
    assert!(error.to_string().contains("response lost"));
}

#[tokio::test]
async fn list_subagents_missing_backend_is_an_error() {
    let result = xai_tool_runtime::Tool::run(
        &ListSubagentsTool,
        test_ctx(Resources::new().into_shared()),
        ListSubagentsInput {},
    )
    .await;
    assert!(result.is_err(), "missing backend must not return an empty inventory");
}

#[tokio::test]
async fn list_subagents_successful_empty_inventory() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut resources = Resources::new();
    resources.insert(ChannelBackend::for_session(tx, "root").into_resource());
    let coordinator = async move {
        let Some(SubagentEvent::ListOwned(request)) = rx.recv().await else {
            panic!("expected a list-owned request");
        };
        assert_eq!(request.parent_session_id.as_deref(), Some("root"));
        request.respond_to.send(Vec::new()).unwrap();
    };
    let run = xai_tool_runtime::Tool::run(
        &ListSubagentsTool,
        test_ctx(resources.into_shared()),
        ListSubagentsInput {},
    );
    let (result, ()) = tokio::join!(run, coordinator);
    let output = result.expect("confirmed empty inventory succeeds");
    assert!(output.subagents.is_empty());
    assert_eq!(output.to_string(), "This session has no subagents.");
}

#[tokio::test]
async fn list_subagents_observation_wire_preserves_identity_without_inventing_custody() {
    use crate::implementations::grok_build::task::types::{ChildObservation, SubagentSnapshotStatus};
    use crate::implementations::grok_build::task::terminal::{SubagentTerminal, SubagentTerminalState};
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut resources = Resources::new();
    resources.insert(ChannelBackend::for_session(tx, "root").into_resource());
    let coordinator = async move {
        let Some(SubagentEvent::ListOwned(request)) = rx.recv().await else {
            panic!("expected list-owned request");
        };
        assert_eq!(request.parent_session_id.as_deref(), Some("root"));
        let inspections = [
            ("resident", ChildObservation::Resident { status: SubagentSnapshotStatus::Initializing, duration_ms: 123 }),
            ("terminal", ChildObservation::Terminal(SubagentTerminal {
                attempt_id: "attempt-1".into(),
                state: SubagentTerminalState::Completed,
                output: "actual output".into(),
                tool_calls: 3, turns: 2, duration_ms: 456, tokens_used: 37,
            })),
            ("uncustodied", ChildObservation::Uncustodied),
        ].into_iter().map(|(id, observation)| SubagentInspection {
            subagent_id: id.into(), description: "fixed description".into(),
            subagent_type: "explore".into(), started_at_epoch_ms: 1000,
            persona: Some("reviewer".into()), parent_session_id: "root".into(),
            child_session_id: "child-session".into(), fork_parent_prompt_id: Some("prompt".into()),
            resumed_from: Some("previous".into()), observation,
        }).collect();
        request.respond_to.send(inspections).unwrap();
    };
    let run = xai_tool_runtime::Tool::run(
        &ListSubagentsTool, test_ctx(resources.into_shared()), ListSubagentsInput {},
    );
    let (result, ()) = tokio::join!(run, coordinator);
    let output = result.expect("confirmed inventory");
    let wire = serde_json::to_value(&output).unwrap();
    assert_eq!(wire, serde_json::json!({"subagents": [
        {"subagent_id":"resident", "description":"fixed description", "subagent_type":"explore",
         "started_at_epoch_ms":1000, "persona":"reviewer", "parent_session_id":"root",
         "child_session_id":"child-session", "fork_parent_prompt_id":"prompt", "resumed_from":"previous",
         "duration_ms":123, "state":{"status":"running"}},
        {"subagent_id":"terminal", "description":"fixed description", "subagent_type":"explore",
         "started_at_epoch_ms":1000, "persona":"reviewer", "parent_session_id":"root",
         "child_session_id":"child-session", "fork_parent_prompt_id":"prompt", "resumed_from":"previous",
         "duration_ms":456, "state":{"status":"completed"}},
        {"subagent_id":"uncustodied", "description":"fixed description", "subagent_type":"explore",
         "started_at_epoch_ms":1000, "persona":"reviewer", "parent_session_id":"root",
         "child_session_id":"child-session", "fork_parent_prompt_id":"prompt", "resumed_from":"previous",
         "unavailable":"custody_unavailable"}
    ]}));
    assert!(output.to_string().contains("uncustodied [explore] \"fixed description\": custody unavailable"));
}
