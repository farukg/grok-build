use super::tests::{harness, request};
use crate::implementations::grok_build::task::backend::SubagentBackend;
use crate::implementations::grok_build::task::terminal::SubagentTerminalState;
use crate::implementations::grok_build::task::types::{
    SubagentActor, SubagentCancelBinding, SubagentCancelDisposition, SubagentCancelOutcome,
};

#[tokio::test]
async fn observed_cancel_refuses_a_different_attempt_without_touching_the_worker() {
    let mut h = harness(false, std::time::Duration::from_secs(60));
    let backend = h.backend.clone();
    let worker = tokio::spawn(async move {
        backend.spawn(request("bound-child", true), None, None).await
    });
    assert_eq!(h.started.recv().await.as_deref(), Some("bound-child"));
    let attempt = h.wake_runs.recv().await.unwrap().1;
    let outcome = h.backend.cancel_with_binding(
        "bound-child", SubagentActor::Human, SubagentCancelDisposition::Stop,
        SubagentCancelBinding::Observed { attempt_id: Some("at1.obsolete".into()) },
    ).await.unwrap();
    assert!(matches!(outcome, SubagentCancelOutcome::CustodyUnavailable { .. }));
    assert!(h.backend.query("bound-child", false, None, None).await.expect("query acknowledgement").unwrap().is_running());
    assert!(h.completions.try_recv().is_err());
    h.finish_one.send("bound-child".into()).unwrap();
    let result = worker.await.unwrap().unwrap();
    assert!(result.success());
    h.completions.recv().await.unwrap();
    let inspection = h.backend.inspect("bound-child").await.expect("coordinator ACK").unwrap();
    let terminal = inspection.terminal.expect("committed worker terminal");
    assert_eq!(terminal.attempt_id, attempt.to_string());
    assert!(matches!(terminal.state, SubagentTerminalState::Completed));
    assert_eq!(terminal.output.as_ref(), "work");
    assert_eq!(terminal.tool_calls, 3);
    assert_eq!(terminal.turns, 2);
    assert_eq!(terminal.duration_ms, result.duration_ms);
    assert_eq!(terminal.tokens_used, result.tokens_used);
    let outcome = h.backend.cancel_with_binding(
        "bound-child", SubagentActor::Human, SubagentCancelDisposition::Stop,
        SubagentCancelBinding::Observed { attempt_id: Some(attempt.to_string()) },
    ).await.unwrap();
    assert!(matches!(outcome, SubagentCancelOutcome::AlreadyFinished { .. }));
    h.actor.abort();
}

#[tokio::test]
async fn cancel_refresh_cannot_bypass_the_producer_commit_barrier() {
    use super::tests::{harness_with_options, ProducerCommitBehavior, RunnerBehavior};
    let mut h = harness_with_options(
        RunnerBehavior {
            producer_commit: ProducerCommitBehavior::HoldAcknowledgement,
            ..Default::default()
        },
        Default::default(),
    );
    let backend = h.backend.clone();
    let worker = tokio::spawn(async move {
        backend.spawn(request("uncommitted", true), None, None).await
    });
    assert_eq!(h.started.recv().await.as_deref(), Some("uncommitted"));
    h.finish_one.send("uncommitted".into()).unwrap();
    let commit = h.terminal_publications.recv().await.unwrap();
    let inspection = h.backend.inspect("uncommitted").await.expect("coordinator ACK").unwrap();
    assert!(inspection.terminal.is_none());
    assert!(inspection.snapshot.is_running());
    commit();
    h.completions.recv().await.unwrap();
    let result = worker.await.unwrap().unwrap();
    assert!(result.success());
    let terminal = h.backend.inspect("uncommitted").await.expect("coordinator ACK").unwrap().terminal.unwrap();
    assert_eq!(terminal.output.as_ref(), "work");
    assert_eq!(terminal.tool_calls, 3);
    assert_eq!(terminal.turns, 2);
    h.actor.abort();
}
