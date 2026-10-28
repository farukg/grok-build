use super::*;
use crate::session::persistence::{PersistenceMsg, native_wake_tests_support::{NativeAdoptionFaultProbe, NativeOwnerTransportCut}};
use xai_grok_tools::implementations::grok_build::task::types::{SubagentActor, SubagentCancelDisposition, SubagentCancelOutcome, ChildCancellation};

async fn flush(owner: &mpsc::UnboundedSender<PersistenceMsg>) {
    let (respond_to, response) = oneshot::channel();
    owner.send(PersistenceMsg::FlushAndAck { respond_to }).unwrap();
    bounded(response).await.unwrap().unwrap();
}

async fn stop(backend: &impl SubagentBackend, reference: &MessageRecoveryRef) -> SubagentCancelOutcome {
    bounded(backend.cancel(&reference.receiver.session_id, SubagentActor::Human, SubagentCancelDisposition::Stop)).await
}

async fn receiver_detached(f: &Fixture) {
    bounded(async {
        loop {
            match f.storage.observe_receiver_custody(&f.info).unwrap().1 {
                crate::session::receiver_execution::ReceiverPresence::Detached => break,
                crate::session::receiver_execution::ReceiverPresence::Live => tokio::task::yield_now().await,
            }
        }
    }).await;
}

async fn cancelled_reopen_cannot_execute(f: &Fixture, before: &crate::session::persistence::Summary, sampled: u32) {
    receiver_detached(f).await;
    let (lease, reopened) = f.storage.claim_receiver(&f.info).await.unwrap();
    let record = &reopened.completion_delivery.receiver.records[&f.reference.delivery_id];
    let original = &before.completion_delivery.receiver.records[&f.reference.delivery_id];
    assert!(matches!(record.execution, DeliveryExecution::Cancelled { .. }));
    assert_eq!(record.parent_recovery_ref(), Some(f.reference.clone()));
    assert_eq!(serde_json::to_value(&record.provenance).unwrap(), serde_json::to_value(&original.provenance).unwrap());
    assert_eq!(record.digest, original.digest);
    assert_eq!(reopened.attempt_id, before.attempt_id);
    assert_eq!(reopened.next_trace_turn, before.next_trace_turn);
    drop(lease);
    let (sender, coordinator_owner) = coordinator(&f.agent, None, None);
    let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
    for action in [MessageRecoveryAction::Observe, MessageRecoveryAction::Adopt] {
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), action)).await,
            ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref: f.reference.clone(), state: MessageRecoveryState::Cancelled });
    }
    assert!(bounded(backend.query(&f.reference.receiver.session_id, false, None, None)).await.is_none());
    assert_eq!(f.server.request_count(), sampled);
    assert_eq!(std::fs::read(f.directory.join("chat_history.jsonl")).unwrap(), f.prior_history);
    assert_eq!(std::fs::read(f.directory.join("child_management.json")).unwrap(), f.prior_recipe);
    let durable = f.storage.load_summary(&f.info).await.unwrap();
    assert!(matches!(durable.completion_delivery.receiver.records[&f.reference.delivery_id].execution, DeliveryExecution::Cancelled { .. }));
    drop(backend);
    drop(coordinator_owner);
}

#[derive(Clone, Copy)]
enum ReleasedCancel {
    ById,
    ParentStop,
    ParentShutdown,
}

async fn cancel_released_adoption(caller: ReleasedCancel) {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner(ParentInputFault::ShutdownBeforeProjection).await;
        let before = f.storage.load_summary(&f.info).await.unwrap();
        let sampled = f.server.request_count();
        let (observe, mut observed) = mpsc::unbounded_channel();
        let (actor, mut actors) = mpsc::unbounded_channel();
        let (sender, coordinator_owner) = coordinator(&f.agent, Some(ParentInputFault::BeforeAdoptedProjection { observe, actor }), None);
        let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await,
            ActiveAgentMessageOutcome::Accepted { message_id: f.reference.message_id.clone() });
        let probe = bounded(observed.recv()).await.unwrap();
        let actor = bounded(actors.recv()).await.unwrap();
        let outcome = match caller {
            ReleasedCancel::ById => stop(&backend, &f.reference).await,
            ReleasedCancel::ParentStop => bounded(backend.cancel_parent_session()).await,
            ReleasedCancel::ParentShutdown => {
                let (respond_to, response) = oneshot::channel();
                assert!(backend.request_cancel_parent_session(ChildCancellation::Shutdown, respond_to));
                bounded(response).await.unwrap()
            }
        };
        assert!(matches!(outcome, SubagentCancelOutcome::Cancelled { .. }));
        bounded(probe.cancellation.cancelled()).await;
        probe.release.send(()).unwrap();
        match caller {
            ReleasedCancel::ById | ReleasedCancel::ParentStop => {
                let result = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.unwrap();
                assert!(matches!(result.status, SubagentSnapshotStatus::Cancelled { .. }));
            }
            ReleasedCancel::ParentShutdown => bounded(async {
                while let Some(snapshot) = backend.query(&f.reference.receiver.session_id, false, None, None).await {
                    match snapshot.status {
                        SubagentSnapshotStatus::Initializing | SubagentSnapshotStatus::Running { .. }
                        | SubagentSnapshotStatus::Finalizing { .. } => tokio::task::yield_now().await,
                        SubagentSnapshotStatus::Completed { .. } | SubagentSnapshotStatus::Failed { .. }
                        | SubagentSnapshotStatus::Cancelled { .. } => panic!("parent cleanup terminalized the original admission"),
                    }
                }
            }).await,
        }
        bounded(actor.cmd_tx.closed()).await;
        flush(&probe.owner).await;
        drop(actor);
        drop(probe.owner);
        drop(backend);
        drop(coordinator_owner);
        match caller {
            ReleasedCancel::ById | ReleasedCancel::ParentStop => cancelled_reopen_cannot_execute(&f, &before, sampled).await,
            ReleasedCancel::ParentShutdown => retained_reopen(&f, &before, sampled).await,
        }
    }).await;
}

async fn retained_reopen(f: &Fixture, before: &crate::session::persistence::Summary, sampled: u32) {
    receiver_detached(f).await;
    let (lease, reopened) = f.storage.claim_receiver(&f.info).await.unwrap();
    assert_eq!(serde_json::to_value(&reopened.completion_delivery.receiver.records[&f.reference.delivery_id]).unwrap(),
        serde_json::to_value(&before.completion_delivery.receiver.records[&f.reference.delivery_id]).unwrap());
    assert!(matches!(reopened.completion_delivery.receiver.records[&f.reference.delivery_id].execution, DeliveryExecution::Queued { .. }));
    assert_eq!(reopened.attempt_id, before.attempt_id);
    assert_eq!(reopened.next_trace_turn, before.next_trace_turn);
    assert_eq!(f.server.request_count(), sampled);
    assert_eq!(std::fs::read(f.directory.join("chat_history.jsonl")).unwrap(), f.prior_history);
    assert_eq!(std::fs::read(f.directory.join("child_management.json")).unwrap(), f.prior_recipe);
    assert_eq!(std::fs::read(f.artifacts.join("output.json")).unwrap(), f.prior_output);
    drop(lease);
}

#[tokio::test(flavor = "current_thread")]
async fn native_backend_stop_released_adoption_before_projection_is_durably_cancelled() {
    cancel_released_adoption(ReleasedCancel::ById).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_parent_session_stop_released_adoption_is_durably_cancelled() {
    cancel_released_adoption(ReleasedCancel::ParentStop).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_parent_session_shutdown_released_adoption_retains_original_admission() {
    cancel_released_adoption(ReleasedCancel::ParentShutdown).await;
}

#[derive(Clone, Copy)]
enum CancelCaller {
    Coordinator,
    Actor,
    CleanupThenLateStop,
}

async fn reserved_cancel_with_owner_transport_paused(caller: CancelCaller) {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner(ParentInputFault::ShutdownBeforeProjection).await;
        let before = f.storage.load_summary(&f.info).await.unwrap();
        let sampled = f.server.request_count();
        let (observe, mut observed) = mpsc::unbounded_channel();
        let (actor, mut actors) = mpsc::unbounded_channel();
        let (commit, mut commits) = mpsc::unbounded_channel();
        let (sender, coordinator_owner) = coordinator(&f.agent, Some(ParentInputFault::PublishedReconfirm { observe, actor, commit }), None);
        let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
        let mut adoption = Box::pin(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt));
        let (probe, actor) = bounded(async {
            tokio::select! {
                biased;
                pair = async { (observed.recv().await.unwrap(), actors.recv().await.unwrap()) } => pair,
                outcome = &mut adoption => panic!("adoption passed the real reconfirm cut: {outcome:?}"),
            }
        }).await;
        let NativeAdoptionFaultProbe { owner, release, cancellation, pause_owner } = probe;
        let (reached, paused) = oneshot::channel();
        let (resume, resumed) = oneshot::channel();
        pause_owner.send(NativeOwnerTransportCut { reached, release: resumed }).unwrap();
        match caller {
            CancelCaller::Coordinator => {
                assert!(matches!(stop(&backend, &f.reference).await, SubagentCancelOutcome::Cancelled { .. }));
            }
            CancelCaller::Actor => actor.cmd_tx.send(SessionCommand::Cancel(crate::session::CancelOptions {
                cancel_subagents: true, kill_background_tasks: true,
                trigger: Some(crate::session::CancelTrigger::Esc),
                ..Default::default()
            })).unwrap(),
            CancelCaller::CleanupThenLateStop => actor.cmd_tx.send(SessionCommand::Cancel(crate::session::CancelOptions {
                cancel_subagents: true, kill_background_tasks: true,
                trigger: Some(crate::session::CancelTrigger::Shutdown),
                ..Default::default()
            })).unwrap(),
        }
        bounded(cancellation.cancelled()).await;
        bounded(paused).await.unwrap();
        assert_eq!(bounded(&mut adoption).await,
            ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref: f.reference.clone() });
        drop(adoption);
        drop(release);
        assert!(matches!(bounded(commits.recv()).await,
            Some(crate::session::persistence::native_wake_tests_support::NativeDeferredCommit::GateDropped)));
        match caller {
            CancelCaller::Coordinator | CancelCaller::Actor => {}
            CancelCaller::CleanupThenLateStop => {
                assert!(matches!(stop(&backend, &f.reference).await, SubagentCancelOutcome::NotFound));
            }
        }
        resume.send(()).unwrap();
        bounded(actor.cmd_tx.closed()).await;
        flush(&owner).await;
        drop(actor);
        drop(owner);
        drop(backend);
        drop(coordinator_owner);
        match caller {
            CancelCaller::Coordinator | CancelCaller::Actor => cancelled_reopen_cannot_execute(&f, &before, sampled).await,
            CancelCaller::CleanupThenLateStop => retained_reopen(&f, &before, sampled).await,
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_backend_stop_reserved_adoption_survives_gate_drop_before_owner_delivery() {
    reserved_cancel_with_owner_transport_paused(CancelCaller::Coordinator).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_actor_stop_reserved_adoption_survives_gate_drop_before_owner_delivery() {
    reserved_cancel_with_owner_transport_paused(CancelCaller::Actor).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_abandoned_adoption_rejects_late_backend_stop_without_terminal_authority() {
    reserved_cancel_with_owner_transport_paused(CancelCaller::CleanupThenLateStop).await;
}

#[derive(Clone, Copy)]
enum ReleasedEndOrdering {
    StopBeforeTerminalDecision,
    StopAfterRunnerSnapshot,
}

async fn released_attempt_end(ordering: ReleasedEndOrdering) {
    use crate::session::persistence::native_wake_tests_support::NativeAttemptEndCut;
    use crate::session::receiver_execution::InitialMessageTerminalOwnership;
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner(ParentInputFault::ShutdownBeforeProjection).await;
        let before = f.storage.load_summary(&f.info).await.unwrap();
        let sampled = f.server.request_count();
        let (observe, mut observed) = mpsc::unbounded_channel();
        let (actor, mut actors) = mpsc::unbounded_channel();
        let (terminal, mut cuts) = mpsc::unbounded_channel();
        let (sender, coordinator_owner) = coordinator(&f.agent, Some(ParentInputFault::ReleasedAttemptEnd { observe, actor, terminal }), None);
        let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await,
            ActiveAgentMessageOutcome::Accepted { message_id: f.reference.message_id.clone() });
        let probe = bounded(observed.recv()).await.unwrap();
        let actor = bounded(actors.recv()).await.unwrap();
        actor.cmd_tx.send(SessionCommand::Cancel(crate::session::CancelOptions {
            cancel_subagents: true, kill_background_tasks: true,
            trigger: Some(crate::session::CancelTrigger::Shutdown),
            ..Default::default()
        })).unwrap();
        bounded(probe.cancellation.cancelled()).await;
        probe.release.send(()).unwrap();
        let NativeAttemptEndCut::BeforeTerminal { release } = bounded(cuts.recv()).await.unwrap() else {
            panic!("real terminal decision must precede runner snapshot");
        };
        match ordering {
            ReleasedEndOrdering::StopBeforeTerminalDecision => {
                assert!(matches!(stop(&backend, &f.reference).await, SubagentCancelOutcome::Cancelled { .. }));
            }
            ReleasedEndOrdering::StopAfterRunnerSnapshot => {}
        }
        release.send(()).unwrap();
        let NativeAttemptEndCut::RunnerSnapshot { ownership, release } = bounded(cuts.recv()).await.unwrap() else {
            panic!("runner must snapshot the completed real attempt");
        };
        let snapshot = bounded(backend.query(&f.reference.receiver.session_id, false, None, None)).await.unwrap();
        assert!(matches!(snapshot.status, SubagentSnapshotStatus::Initializing | SubagentSnapshotStatus::Running { .. }
            | SubagentSnapshotStatus::Finalizing { .. }));
        match ordering {
            ReleasedEndOrdering::StopBeforeTerminalDecision => assert!(matches!(ownership, InitialMessageTerminalOwnership::Message)),
            ReleasedEndOrdering::StopAfterRunnerSnapshot => {
                assert!(matches!(ownership, InitialMessageTerminalOwnership::AdoptionAttempt));
                assert!(matches!(stop(&backend, &f.reference).await, SubagentCancelOutcome::NotFound));
            }
        }
        release.send(()).unwrap();
        match ordering {
            ReleasedEndOrdering::StopBeforeTerminalDecision => {
                let result = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.unwrap();
                assert!(matches!(result.status, SubagentSnapshotStatus::Cancelled { .. }));
            }
            ReleasedEndOrdering::StopAfterRunnerSnapshot => bounded(async {
                while let Some(snapshot) = backend.query(&f.reference.receiver.session_id, false, None, None).await {
                    match snapshot.status {
                        SubagentSnapshotStatus::Initializing | SubagentSnapshotStatus::Running { .. }
                        | SubagentSnapshotStatus::Finalizing { .. } => tokio::task::yield_now().await,
                        SubagentSnapshotStatus::Completed { .. } | SubagentSnapshotStatus::Failed { .. }
                        | SubagentSnapshotStatus::Cancelled { .. } => panic!("ended adopter changed the original admission"),
                    }
                }
            }).await,
        }
        bounded(actor.cmd_tx.closed()).await;
        flush(&probe.owner).await;
        drop(actor);
        drop(probe.owner);
        drop(backend);
        drop(coordinator_owner);
        match ordering {
            ReleasedEndOrdering::StopBeforeTerminalDecision => cancelled_reopen_cannot_execute(&f, &before, sampled).await,
            ReleasedEndOrdering::StopAfterRunnerSnapshot => retained_reopen(&f, &before, sampled).await,
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_released_attempt_end_rejects_backend_stop_after_real_runner_snapshot() {
    released_attempt_end(ReleasedEndOrdering::StopAfterRunnerSnapshot).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_released_attempt_stop_before_terminal_decision_is_durably_cancelled() {
    released_attempt_end(ReleasedEndOrdering::StopBeforeTerminalDecision).await;
}
