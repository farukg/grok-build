use super::*;
use super::tests::{completed_subagent_snapshot, resources_with_backend_query, unwrap_query};
use crate::implementations::grok_build::task::types::{SubagentResult, SubagentState};
use crate::types::completion_observation::{ObservedCompletion, SubagentCompletionObservations};
use crate::types::tool_metadata::test_ctx;
use xai_message_delivery_core::AttemptId;

fn record_completion(observations: &SubagentCompletionObservations) {
    observations.record(&SubagentResult {
        state: SubagentState::Completed,
        subagent_id: "done".into(),
        output: "Found 3 files".into(),
        tool_calls: 5,
        turns: 2,
        ..Default::default()
    }, &AttemptId::mint(1));
}

#[tokio::test]
async fn query_closed_channel_is_a_tool_error() {
    let (resources, rx) = resources_with_backend_query();
    drop(rx);
    let result = xai_tool_runtime::Tool::run(&TaskOutputTool, test_ctx(resources.into_shared()), TaskOutputToolInput {
        task_ids: vec!["child".into()], timeout_ms: None,
    }).await;
    let error = result.unwrap_err().to_string();
    assert!(error.contains("coordinator unavailable"), "{error}");
}

#[tokio::test]
async fn query_lost_ack_does_not_publish_observations() {
    let (resources, mut rx) = resources_with_backend_query();
    let observations = SubagentCompletionObservations::default();
    let mut ctx = test_ctx(resources.into_shared());
    ctx.extensions.insert(observations.clone());
    let responder = async {
        let request = unwrap_query(rx.recv().await.unwrap());
        assert_eq!(request.subagent_id, "child");
        record_completion(request.observations.as_ref().unwrap());
        drop(request.respond_to);
    };
    let (result, ()) = tokio::join!(xai_tool_runtime::Tool::run(&TaskOutputTool, ctx, TaskOutputToolInput {
        task_ids: vec!["child".into()], timeout_ms: None,
    }), responder);
    assert!(result.unwrap_err().to_string().contains("coordinator unavailable"));
    assert!(observations.snapshot().is_empty());
}

#[tokio::test]
async fn multi_query_error_does_not_consume_successful_output() {
    for lose_ack in [false, true] {
        let (resources, mut rx) = resources_with_backend_query();
        let observations = SubagentCompletionObservations::default();
        let mut ctx = test_ctx(resources.into_shared());
        ctx.extensions.insert(observations.clone());
        let responder = async {
            let request = unwrap_query(rx.recv().await.unwrap());
            assert_eq!(request.subagent_id, "done");
            record_completion(request.observations.as_ref().unwrap());
            request.respond_to.send(Some(completed_subagent_snapshot("done"))).unwrap();
            if lose_ack {
                let request = unwrap_query(rx.recv().await.unwrap());
                assert_eq!(request.subagent_id, "lost");
                drop(request.respond_to);
            }
            drop(rx);
        };
        let (result, ()) = tokio::join!(xai_tool_runtime::Tool::run(&TaskOutputTool, ctx, TaskOutputToolInput {
            task_ids: vec!["done".into(), "lost".into()], timeout_ms: Some(1000),
        }), responder);
        assert!(result.unwrap_err().to_string().contains("coordinator unavailable"));
        assert!(observations.snapshot().is_empty(), "failed tool output was not delivered");
    }
}

#[tokio::test]
async fn multi_wait_failure_does_not_publish_initial_completions() {
    for fail_wait in [false, true] {
        let (resources, mut rx) = resources_with_backend_query();
        let observations = SubagentCompletionObservations::default();
        let mut ctx = test_ctx(resources.into_shared());
        ctx.extensions.insert(observations.clone());
        let responder = async {
            let request = unwrap_query(rx.recv().await.unwrap());
            assert_eq!(request.subagent_id, "done");
            record_completion(request.observations.as_ref().unwrap());
            request.respond_to.send(Some(completed_subagent_snapshot("done"))).unwrap();
            let request = unwrap_query(rx.recv().await.unwrap());
            assert_eq!(request.subagent_id, "running");
            let mut snapshot = completed_subagent_snapshot("running");
            snapshot.status = SubagentSnapshotStatus::Initializing;
            request.respond_to.send(Some(snapshot)).unwrap();
            let request = unwrap_query(rx.recv().await.unwrap());
            assert_eq!(request.subagent_id, "running");
            assert!(request.block);
            if fail_wait {
                drop(request.respond_to);
            } else {
                request.respond_to.send(Some(completed_subagent_snapshot("running"))).unwrap();
                let request = unwrap_query(rx.recv().await.unwrap());
                assert_eq!(request.subagent_id, "done");
                record_completion(request.observations.as_ref().unwrap());
                request.respond_to.send(Some(completed_subagent_snapshot("done"))).unwrap();
                let request = unwrap_query(rx.recv().await.unwrap());
                assert_eq!(request.subagent_id, "running");
                assert!(!request.block);
                drop(request.respond_to);
            }
        };
        let (result, ()) = tokio::join!(xai_tool_runtime::Tool::run(&TaskOutputTool, ctx, TaskOutputToolInput {
            task_ids: vec!["done".into(), "running".into()], timeout_ms: Some(1000),
        }), responder);
        assert!(result.unwrap_err().to_string().contains("coordinator unavailable"));
        assert!(observations.snapshot().is_empty(), "initial snapshots were not delivered");
    }
}

#[tokio::test]
async fn multi_query_success_publishes_exactly_one_observation() {
    let (resources, mut rx) = resources_with_backend_query();
    let observations = SubagentCompletionObservations::default();
    let mut ctx = test_ctx(resources.into_shared());
    ctx.extensions.insert(observations.clone());
    let responder = async {
        let request = unwrap_query(rx.recv().await.unwrap());
        record_completion(request.observations.as_ref().unwrap());
        request.respond_to.send(Some(completed_subagent_snapshot("done"))).unwrap();
        let request = unwrap_query(rx.recv().await.unwrap());
        assert_eq!(request.subagent_id, "missing");
        request.respond_to.send(None).unwrap();
    };
    let (result, ()) = tokio::join!(xai_tool_runtime::Tool::run(&TaskOutputTool, ctx, TaskOutputToolInput {
        task_ids: vec!["done".into(), "missing".into()], timeout_ms: None,
    }), responder);
    let TaskOutputOutput::MultiResult(result) = result.unwrap() else { panic!("expected multi result") };
    assert_eq!(result.results[0].status, "completed");
    assert!(result.results[0].output.contains("Found 3 files"));
    assert_eq!(result.results[1].status, "not_found");
    let observed = observations.snapshot();
    let [ObservedCompletion::Subagent(completion)] = observed.as_slice() else { panic!("expected one delivered completion") };
    assert_eq!(completion.subagent_id, "done");
    assert_eq!(completion.attempt_id, AttemptId::mint(1));
    assert!(completion.text.contains("Found 3 files"));
}

#[tokio::test]
async fn blocking_multi_query_loss_is_not_completion() {
    for mode in [xai_tool_types::WaitMode::WaitAll, xai_tool_types::WaitMode::WaitAny] {
        let (resources, mut rx) = resources_with_backend_query();
        let responder = async {
            let request = unwrap_query(rx.recv().await.unwrap());
            assert!(!request.block);
            let mut snapshot = completed_subagent_snapshot("running");
            snapshot.status = SubagentSnapshotStatus::Initializing;
            request.respond_to.send(Some(snapshot)).unwrap();
            let request = unwrap_query(rx.recv().await.unwrap());
            assert_eq!(request.subagent_id, "missing");
            request.respond_to.send(None).unwrap();
            let request = unwrap_query(rx.recv().await.unwrap());
            assert_eq!(request.subagent_id, "running");
            assert!(request.block);
            assert!(request.timeout_ms.is_some_and(|ms| ms > 0 && ms <= 1000));
            drop(request.respond_to);
        };
        let (result, ()) = tokio::join!(xai_tool_runtime::Tool::run(&WaitTasksTool, test_ctx(resources.into_shared()), xai_tool_types::WaitTasksToolInput {
            task_ids: vec!["running".into(), "missing".into()], timeout_ms: Some(1000), mode,
        }), responder);
        assert!(result.unwrap_err().to_string().contains("coordinator unavailable"));
        assert!(rx.try_recv().is_err(), "failed wait must not trigger a success refresh");
    }
}
