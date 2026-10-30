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
