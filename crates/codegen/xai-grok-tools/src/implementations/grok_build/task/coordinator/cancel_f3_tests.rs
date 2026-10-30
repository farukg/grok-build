use super::tests::{harness, request};
use crate::implementations::grok_build::task::backend::SubagentBackend;
use crate::implementations::grok_build::task::terminal::SubagentTerminalState;
use crate::implementations::grok_build::task::types::{
    ChildObservation, SubagentSnapshotStatus, SubagentActor, SubagentCancelBinding, SubagentCancelDisposition, SubagentCancelOutcome,
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
    let ChildObservation::Terminal(terminal) = inspection.observation else {
        panic!("expected committed worker terminal");
    };
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
    assert!(matches!(&inspection.observation, ChildObservation::Resident { status: SubagentSnapshotStatus::Finalizing { .. }, .. }));
    assert!(inspection.snapshot().expect("resident snapshot").is_running());
    commit();
    h.completions.recv().await.unwrap();
    let result = worker.await.unwrap().unwrap();
    assert!(result.success());
    let ChildObservation::Terminal(terminal) = h.backend.inspect("uncommitted").await.expect("coordinator ACK").unwrap().observation else {
        panic!("expected committed terminal");
    };
    assert_eq!(terminal.output.as_ref(), "work");
    assert_eq!(terminal.tool_calls, 3);
    assert_eq!(terminal.turns, 2);
    h.actor.abort();
}

async fn inspect_record(
    coordinator: &mut super::active_message::tests::TestCoordinator,
) -> crate::implementations::grok_build::task::types::SubagentInspection {
    let (tx, rx) = tokio::sync::oneshot::channel();
    coordinator.handle_inspect("evidence".into(), Some("parent".into()), tx);
    rx.await.expect("inspection ACK").expect("owned record")
}

#[tokio::test]
async fn inspection_publication_and_spill_failure_require_full_producer_evidence() {
    use super::active_message::tests::{fixture, insert_child, finish_child_with_result_before_commit};
    use crate::implementations::grok_build::task::coordinator_state::{
        TerminalPublication, TerminalPublicationState, TerminalCommitResult, OUTPUT_UNAVAILABLE_PLACEHOLDER,
    };
    use crate::implementations::grok_build::task::types::{SubagentResult, SubagentState, SubagentFinalization};
    for commit_result in [TerminalCommitResult::Committed, TerminalCommitResult::Rejected { error: "commit rejected".into() }] {
        let (mut coordinator, _commands, admissions, _calls) = fixture();
        insert_child(&mut coordinator, admissions, "evidence", "parent");
        let attempt = coordinator.active["evidence"].attempt_id.clone();
        finish_child_with_result_before_commit(&mut coordinator, "evidence", SubagentResult {
            subagent_id: "evidence".into(), child_session_id: "evidence".into(),
            state: SubagentState::Completed, output: "full producer output".into(),
            tool_calls: 17, turns: 9, tokens_used: 71, duration_ms: 321,
            ..Default::default()
        });
        let inspection = inspect_record(&mut coordinator).await;
        assert!(matches!(inspection.observation, ChildObservation::Resident {
            status: SubagentSnapshotStatus::Finalizing { publication: SubagentFinalization::Writing },
            duration_ms: 321,
        }));
        let prepared = coordinator.terminal_published_rx.try_recv().expect("producer commit callback");
        if let TerminalCommitResult::Rejected { error } = commit_result {
            assert!(matches!(prepared, TerminalPublication::Prepared { .. }));
            coordinator.handle_terminal_publication(TerminalPublication::Prepared {
                subagent_id: "evidence".into(), attempt_id: attempt,
                result: TerminalCommitResult::Rejected { error },
            });
            assert!(matches!(inspect_record(&mut coordinator).await.observation,
                ChildObservation::Resident {
                    status: SubagentSnapshotStatus::Finalizing { publication: SubagentFinalization::Blocked { error } },
                    duration_ms: 321,
                } if error == "commit rejected"));
            continue;
        }
        coordinator.handle_terminal_publication(prepared);
        for publication in [TerminalPublicationState::Publishing, TerminalPublicationState::Published] {
            if matches!(publication, TerminalPublicationState::Published) {
                let visible = coordinator.terminal_published_rx.try_recv().expect("visibility callback");
                coordinator.handle_terminal_publication(visible);
            }
            let ChildObservation::Terminal(terminal) = inspect_record(&mut coordinator).await.observation else {
                panic!("publishing and published retain full evidence");
            };
            assert_eq!(terminal.attempt_id, attempt.to_string());
            assert!(matches!(terminal.state, SubagentTerminalState::Completed));
            assert_eq!(terminal.output.as_ref(), "full producer output");
            assert_eq!(terminal.tool_calls, 17);
            assert_eq!(terminal.turns, 9);
            assert_eq!(terminal.tokens_used, 71);
            assert_eq!(terminal.duration_ms, 321);
            if matches!(publication, TerminalPublicationState::Publishing) {
                coordinator.completed.get_mut("evidence").unwrap().persisted_output_ref = Some("missing-spill".into());
                let inspection = inspect_record(&mut coordinator).await;
                assert!(matches!(inspection.observation, ChildObservation::Resident {
                    status: SubagentSnapshotStatus::Completed { output, tool_calls: 17, turns: 9, .. },
                    duration_ms: 321,
                } if output == OUTPUT_UNAVAILABLE_PLACEHOLDER));
                coordinator.completed.get_mut("evidence").unwrap().persisted_output_ref = None;
            }
        }
    }
}

#[tokio::test]
async fn inspection_history_blocked_has_no_terminal_evidence() {
    use super::active_message::tests::{fixture, insert_child, finish_child_before_visibility};
    use crate::implementations::grok_build::task::coordinator_state::{TerminalPublication, TerminalVisibilityResult};
    use crate::implementations::grok_build::task::types::SubagentFinalization;
    let (mut coordinator, _commands, admissions, _calls) = fixture();
    insert_child(&mut coordinator, admissions, "evidence", "parent");
    let attempt = coordinator.active["evidence"].attempt_id.clone();
    finish_child_before_visibility(&mut coordinator, "evidence");
    let _visible = coordinator.terminal_published_rx.try_recv().expect("visibility callback");
    coordinator.handle_terminal_publication(TerminalPublication::Visible {
        subagent_id: "evidence".into(), attempt_id: attempt,
        result: TerminalVisibilityResult::Deferred { error: "history blocked".into() },
    });
    assert!(matches!(inspect_record(&mut coordinator).await.observation,
        ChildObservation::Resident {
            status: SubagentSnapshotStatus::Finalizing { publication: SubagentFinalization::Blocked { error } }, ..
        } if error == "history blocked"));
}
