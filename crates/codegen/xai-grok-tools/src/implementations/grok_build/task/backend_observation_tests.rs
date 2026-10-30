use super::*;
use crate::implementations::grok_build::task::types::{SubagentAccessError, SubagentSnapshotStatus};
use crate::implementations::grok_build::task::terminal::{SubagentTerminal, SubagentTerminalState};
use tokio::sync::mpsc;

async fn confirmed<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(1), future)
        .await
        .expect("observation must not hang")
}

fn inspection() -> SubagentInspection {
    SubagentInspection {
        subagent_id: "child".into(),
        description: "inspect work".into(),
        subagent_type: "explore".into(),
        started_at_epoch_ms: 1000,
        persona: Some("reviewer".into()),
        observation: crate::implementations::grok_build::task::types::ChildObservation::Terminal(SubagentTerminal {
            attempt_id: "attempt-1".into(),
            state: SubagentTerminalState::Completed,
            output: Arc::from("real output"),
            tool_calls: 3,
            turns: 2,
            duration_ms: 200,
            tokens_used: 37,
        }),
        parent_session_id: "root".into(),
        child_session_id: "child-session".into(),
        fork_parent_prompt_id: Some("prompt".into()),
        resumed_from: Some("previous".into()),
    }
}

fn assert_inspection(child: SubagentInspection) {
    assert_eq!(child.subagent_id, "child");
    assert_eq!(child.description, "inspect work");
    assert_eq!(child.subagent_type, "explore");
    assert_eq!(child.started_at_epoch_ms, 1000);
    assert_eq!(child.snapshot().expect("available snapshot").duration_ms, 200);
    assert_eq!(child.persona.as_deref(), Some("reviewer"));
    assert!(matches!(child.snapshot().expect("available snapshot").status,
        SubagentSnapshotStatus::Completed { output, tool_calls: 3, turns: 2, worktree_path: None }
        if output == "real output"
    ));
    let crate::implementations::grok_build::task::types::ChildObservation::Terminal(terminal) = &child.observation else {
        panic!("expected real terminal evidence");
    };
    assert_eq!(terminal.attempt_id, "attempt-1");
    assert!(matches!(terminal.state, SubagentTerminalState::Completed));
    assert_eq!(terminal.output.as_ref(), "real output");
    assert_eq!(terminal.tool_calls, 3);
    assert_eq!(terminal.turns, 2);
    assert_eq!(terminal.duration_ms, 200);
    assert_eq!(terminal.tokens_used, 37);
    assert_eq!(child.parent_session_id, "root");
    assert_eq!(child.child_session_id, "child-session");
    assert_eq!(child.fork_parent_prompt_id.as_deref(), Some("prompt"));
    assert_eq!(child.resumed_from.as_deref(), Some("previous"));
}

#[tokio::test]
async fn channel_backend_inspect_closed_channel() {
    let (tx, rx) = mpsc::unbounded_channel::<SubagentEvent>();
    drop(rx);
    let backend = ChannelBackend::for_session(tx, "root");
    assert!(matches!(confirmed(backend.inspect("child")).await,
        Err(SubagentAccessError::CoordinatorUnavailable)));
}

#[tokio::test]
async fn channel_backend_inspect_lost_ack() {
    let (tx, mut rx) = mpsc::unbounded_channel::<SubagentEvent>();
    let backend = ChannelBackend::for_session(tx, "root");
    let receiver = tokio::spawn(async move {
        let Some(SubagentEvent::Inspect(request)) = rx.recv().await else {
            panic!("expected inspect request");
        };
        assert_eq!(request.subagent_id, "child");
        assert_eq!(request.parent_session_id.as_deref(), Some("root"));
        drop(request.respond_to);
    });
    assert!(matches!(confirmed(backend.inspect("child")).await,
        Err(SubagentAccessError::CoordinatorUnavailable)));
    receiver.await.unwrap();
}

#[tokio::test]
async fn channel_backend_inspect_miss() {
    let (tx, mut rx) = mpsc::unbounded_channel::<SubagentEvent>();
    let backend = ChannelBackend::for_session(tx, "root");
    let receiver = tokio::spawn(async move {
        let Some(SubagentEvent::Inspect(request)) = rx.recv().await else {
            panic!("expected inspect request");
        };
        assert_eq!(request.subagent_id, "missing");
        assert_eq!(request.parent_session_id.as_deref(), Some("root"));
        request.respond_to.send(None).unwrap();
    });
    assert!(confirmed(backend.inspect("missing")).await.unwrap().is_none());
    receiver.await.unwrap();
}

#[tokio::test]
async fn channel_backend_inspect_success() {
    let (tx, mut rx) = mpsc::unbounded_channel::<SubagentEvent>();
    let backend = ChannelBackend::for_session(tx, "root");
    let receiver = tokio::spawn(async move {
        let Some(SubagentEvent::Inspect(request)) = rx.recv().await else {
            panic!("expected inspect request");
        };
        assert_eq!(request.subagent_id, "child");
        assert_eq!(request.parent_session_id.as_deref(), Some("root"));
        request.respond_to.send(Some(inspection())).unwrap();
    });
    assert_inspection(confirmed(backend.inspect("child")).await.unwrap().unwrap());
    receiver.await.unwrap();
}

#[tokio::test]
async fn channel_backend_list_running_closed_channel() {
    let (tx, rx) = mpsc::unbounded_channel::<SubagentEvent>();
    drop(rx);
    let backend = ChannelBackend::for_session(tx, "root");
    assert!(matches!(confirmed(backend.list_running("root")).await,
        Err(SubagentAccessError::CoordinatorUnavailable)));
}

#[tokio::test]
async fn channel_backend_list_running_lost_ack() {
    let (tx, mut rx) = mpsc::unbounded_channel::<SubagentEvent>();
    let backend = ChannelBackend::for_session(tx, "root");
    let receiver = tokio::spawn(async move {
        let Some(SubagentEvent::ListRunning(request)) = rx.recv().await else {
            panic!("expected list running request");
        };
        assert_eq!(request.parent_session_id, "root");
        drop(request.respond_to);
    });
    assert!(matches!(confirmed(backend.list_running("root")).await,
        Err(SubagentAccessError::CoordinatorUnavailable)));
    receiver.await.unwrap();
}

#[tokio::test]
async fn channel_backend_list_running_empty() {
    let (tx, mut rx) = mpsc::unbounded_channel::<SubagentEvent>();
    let backend = ChannelBackend::for_session(tx, "root");
    let receiver = tokio::spawn(async move {
        let Some(SubagentEvent::ListRunning(request)) = rx.recv().await else {
            panic!("expected list running request");
        };
        assert_eq!(request.parent_session_id, "root");
        request.respond_to.send(Vec::new()).unwrap();
    });
    assert!(confirmed(backend.list_running("root")).await.unwrap().is_empty());
    receiver.await.unwrap();
}

#[tokio::test]
async fn channel_backend_list_running_success() {
    let (tx, mut rx) = mpsc::unbounded_channel::<SubagentEvent>();
    let backend = ChannelBackend::for_session(tx, "root");
    let receiver = tokio::spawn(async move {
        let Some(SubagentEvent::ListRunning(request)) = rx.recv().await else {
            panic!("expected list running request");
        };
        assert_eq!(request.parent_session_id, "root");
        request.respond_to.send(vec![inspection()]).unwrap();
    });
    let mut listed = confirmed(backend.list_running("root")).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_inspection(listed.pop().unwrap());
    receiver.await.unwrap();
}

#[test]
fn observation_snapshot_is_partial_and_preserves_resident_record_values() {
    use crate::implementations::grok_build::task::types::ChildObservation;
    let mut child = inspection();
    child.observation = ChildObservation::Uncustodied;
    assert!(child.snapshot().is_none());
    child.observation = ChildObservation::Resident {
        status: SubagentSnapshotStatus::Failed { error: "record error".into() },
        duration_ms: 987,
    };
    let snapshot = child.snapshot().expect("resident snapshot");
    assert_eq!(snapshot.duration_ms, 987);
    assert_eq!(snapshot.subagent_id, "child");
    assert_eq!(snapshot.description, "inspect work");
    assert_eq!(snapshot.subagent_type, "explore");
    assert_eq!(snapshot.started_at_epoch_ms, 1000);
    assert_eq!(snapshot.persona.as_deref(), Some("reviewer"));
    assert!(matches!(snapshot.status, SubagentSnapshotStatus::Failed { error } if error == "record error"));
}

#[test]
fn failed_and_cancelled_snapshot_does_not_replace_full_terminal_evidence() {
    use crate::implementations::grok_build::task::types::{ChildObservation, InterruptionCause};
    for state in [
        SubagentTerminalState::Failed { message: "producer error".into() },
        SubagentTerminalState::Interrupted { cause: InterruptionCause::ProcessRestart },
    ] {
        let mut child = inspection();
        let ChildObservation::Terminal(terminal) = &mut child.observation else {
            panic!("terminal fixture");
        };
        terminal.state = state;
        let snapshot = child.snapshot().expect("terminal projection");
        assert_eq!(snapshot.duration_ms, 200);
        let ChildObservation::Terminal(terminal) = &child.observation else {
            panic!("terminal evidence retained");
        };
        match &terminal.state {
            SubagentTerminalState::Failed { message } => {
                assert_eq!(message, "producer error");
                assert!(matches!(snapshot.status, SubagentSnapshotStatus::Failed { error } if error == "producer error"));
            }
            SubagentTerminalState::Interrupted { cause } => {
                assert_eq!(cause, &InterruptionCause::ProcessRestart);
                assert!(matches!(snapshot.status, SubagentSnapshotStatus::Cancelled { cause: InterruptionCause::ProcessRestart }));
            }
            SubagentTerminalState::Completed => panic!("non-completed fixture"),
        }
        assert_eq!(terminal.attempt_id, "attempt-1");
        assert_eq!(terminal.output.as_ref(), "real output");
        assert_eq!(terminal.tool_calls, 3);
        assert_eq!(terminal.turns, 2);
        assert_eq!(terminal.tokens_used, 37);
        assert_eq!(terminal.duration_ms, 200);
    }
}
