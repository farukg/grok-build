use super::*;
use super::tests::{completed_subagent_snapshot, resources_with_backend_query, unwrap_query};
use crate::implementations::grok_build::task::types::{SubagentResult, SubagentState};
use crate::types::completion_observation::{ObservedCompletion, SubagentCompletionObservations};
use crate::types::tool_metadata::test_ctx;
use xai_message_delivery_core::AttemptId;
use xai_tool_types::{WaitMode, WaitTasksToolInput};

fn record(observations: &SubagentCompletionObservations, id: &str, attempt: u128) {
    observations.record(&SubagentResult {
        state: SubagentState::Completed,
        subagent_id: id.into(),
        output: "Found 3 files".into(),
        tool_calls: 5,
        turns: 2,
        ..Default::default()
    }, &AttemptId::mint(attempt));
}

fn assert_observed(observations: &SubagentCompletionObservations, expected: &[(&str, u128)]) {
    let actual = observations.snapshot();
    assert_eq!(actual.len(), expected.len());
    for (actual, (id, attempt)) in actual.iter().zip(expected) {
        let ObservedCompletion::Subagent(actual) = actual else {
            panic!("expected subagent completion");
        };
        assert_eq!(actual.subagent_id, *id);
        assert_eq!(actual.attempt_id, AttemptId::mint(*attempt));
        assert!(actual.text.contains("Found 3 files"));
    }
}

#[tokio::test]
async fn aliases_publish_only_final_results_and_deduplicate_repeated_attempts() {
    for mode in [WaitMode::WaitAll, WaitMode::WaitAny] {
        let (resources, mut rx) = resources_with_backend_query();
        let observations = SubagentCompletionObservations::default();
        record(&observations, "foreign", 7);
        let mut ctx = test_ctx(resources.into_shared());
        ctx.extensions.insert(observations.clone());
        let responder = async {
            let request = unwrap_query(rx.recv().await.unwrap());
            assert_eq!(request.subagent_id, "done");
            record(request.observations.as_ref().unwrap(), "initial-only", 2);
            request.respond_to.send(Some(completed_subagent_snapshot("done"))).unwrap();
            let request = unwrap_query(rx.recv().await.unwrap());
            let mut running = completed_subagent_snapshot("running");
            running.status = SubagentSnapshotStatus::Initializing;
            request.respond_to.send(Some(running)).unwrap();
            let request = unwrap_query(rx.recv().await.unwrap());
            assert!(request.block);
            assert!(request.observations.is_none());
            request.respond_to.send(Some(completed_subagent_snapshot("running"))).unwrap();
            for id in ["done", "running"] {
                let request = unwrap_query(rx.recv().await.unwrap());
                assert_eq!(request.subagent_id, id);
                assert!(!request.block);
                record(request.observations.as_ref().unwrap(), id, 1);
                record(request.observations.as_ref().unwrap(), id, 1);
                request.respond_to.send(Some(completed_subagent_snapshot(id))).unwrap();
            }
        };
        let (result, ()) = tokio::join!(xai_tool_runtime::Tool::run(&WaitTasksTool, ctx, WaitTasksToolInput {
            task_ids: vec!["done".into(), "running".into()], timeout_ms: Some(1000), mode,
        }), responder);
        let TaskOutputOutput::MultiResult(result) = result.unwrap() else { panic!("expected multi result") };
        assert_eq!(result.mode, match mode {
            WaitMode::WaitAll => "wait_all",
            WaitMode::WaitAny => "wait_any",
        });
        assert_eq!(result.results.len(), 2);
        for (result, id) in result.results.iter().zip(["done", "running"]) {
            assert_eq!(result.task_id, id);
            assert_eq!(result.status, "completed");
            assert!(result.output.contains("Found 3 files"));
        }
        assert_observed(&observations, &[("foreign", 7), ("done", 1), ("running", 1)]);

        let (resources, mut rx) = resources_with_backend_query();
        let mut repeated_ctx = test_ctx(resources.into_shared());
        repeated_ctx.extensions.insert(observations.clone());
        let responder = async {
            for _ in 0..2 {
                let request = unwrap_query(rx.recv().await.unwrap());
                record(request.observations.as_ref().unwrap(), "done", 1);
                request.respond_to.send(Some(completed_subagent_snapshot("done"))).unwrap();
            }
        };
        let (result, ()) = tokio::join!(xai_tool_runtime::Tool::run(&WaitTasksTool, repeated_ctx, WaitTasksToolInput {
            task_ids: vec!["done".into(), "done".into()], timeout_ms: Some(1000), mode,
        }), responder);
        assert!(result.is_ok());
        assert_observed(&observations, &[("foreign", 7), ("done", 1), ("running", 1)]);
    }
}

#[derive(Clone, Copy)]
enum Failure {
    Initial,
    Wait,
    FinalRefresh,
}

#[tokio::test]
async fn alias_errors_preserve_foreign_observations_without_publishing_partial_results() {
    for mode in [WaitMode::WaitAll, WaitMode::WaitAny] {
        for failure in [Failure::Initial, Failure::Wait, Failure::FinalRefresh] {
            let (resources, mut rx) = resources_with_backend_query();
            let observations = SubagentCompletionObservations::default();
            record(&observations, "foreign", 7);
            let mut ctx = test_ctx(resources.into_shared());
            ctx.extensions.insert(observations.clone());
            let responder = async {
                let request = unwrap_query(rx.recv().await.unwrap());
                record(request.observations.as_ref().unwrap(), "done", 1);
                request.respond_to.send(Some(completed_subagent_snapshot("done"))).unwrap();
                let request = unwrap_query(rx.recv().await.unwrap());
                match failure {
                    Failure::Initial => {
                        record(request.observations.as_ref().unwrap(), "undelivered", 3);
                        drop(request.respond_to);
                        return;
                    }
                    Failure::Wait | Failure::FinalRefresh => {}
                }
                let mut running = completed_subagent_snapshot("running");
                running.status = SubagentSnapshotStatus::Initializing;
                request.respond_to.send(Some(running)).unwrap();
                let request = unwrap_query(rx.recv().await.unwrap());
                assert!(request.block);
                assert!(request.observations.is_none());
                match failure {
                    Failure::Wait => drop(request.respond_to),
                    Failure::FinalRefresh => {
                        request.respond_to.send(Some(completed_subagent_snapshot("running"))).unwrap();
                        let request = unwrap_query(rx.recv().await.unwrap());
                        assert_eq!(request.subagent_id, "done");
                        record(request.observations.as_ref().unwrap(), "done", 1);
                        request.respond_to.send(Some(completed_subagent_snapshot("done"))).unwrap();
                        let request = unwrap_query(rx.recv().await.unwrap());
                        assert_eq!(request.subagent_id, "running");
                        record(request.observations.as_ref().unwrap(), "undelivered", 3);
                        drop(request.respond_to);
                    }
                    Failure::Initial => panic!("initial failure already returned"),
                }
            };
            let (result, ()) = tokio::join!(xai_tool_runtime::Tool::run(&WaitTasksTool, ctx, WaitTasksToolInput {
                task_ids: vec!["done".into(), "running".into()], timeout_ms: Some(1000), mode,
            }), responder);
            assert!(result.unwrap_err().to_string().contains("coordinator unavailable"));
            assert_observed(&observations, &[("foreign", 7)]);
        }
    }
}
