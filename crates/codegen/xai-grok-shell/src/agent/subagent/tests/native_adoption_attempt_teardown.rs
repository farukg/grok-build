use super::*;
use crate::session::SessionHandle;
use crate::session::persistence::{
    PersistenceMsg, native_wake_tests_support::NativeAdoptionFaultProbe,
};
use tokio::sync::oneshot;

#[derive(Clone, Copy)]
enum FailedAdoption {
    ReconfirmTimeout,
    LostReconfirmReply,
    DeferredCommitRejected,
}

async fn flush_live_owner(owner: &mpsc::UnboundedSender<PersistenceMsg>) {
    let (respond_to, response) = oneshot::channel();
    owner
        .send(PersistenceMsg::FlushAndAck { respond_to })
        .unwrap();
    bounded(response).await.unwrap().unwrap();
}

async fn await_failed_attempt_teardown(
    backend: &impl SubagentBackend,
    actor: &SessionHandle,
    id: &str,
) {
    bounded(actor.cmd_tx.closed()).await;
    bounded(async {
        loop {
            match backend.query(id, false, None, None).await.expect("query acknowledgement") {
                None => break,
                Some(snapshot) => match snapshot.status {
                    SubagentSnapshotStatus::Initializing
                    | SubagentSnapshotStatus::Running { .. }
                    | SubagentSnapshotStatus::Finalizing { .. } => tokio::task::yield_now().await,
                    SubagentSnapshotStatus::Completed { .. }
                    | SubagentSnapshotStatus::Failed { .. }
                    | SubagentSnapshotStatus::Cancelled { .. } => {
                        panic!("failed published adopter manufactured a terminal result")
                    }
                },
            }
        }
    })
    .await;
}

async fn failed_adopter_retains_original(cut: FailedAdoption) {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let mut f = lost_owner(ParentInputFault::ShutdownBeforeProjection).await;
        let before = f.storage.load_summary(&f.info).await.unwrap();
        let original = serde_json::to_value(&before.completion_delivery.receiver.records[&f.reference.delivery_id]).unwrap();
        let published_meta = std::fs::read(f.artifacts.join("meta.json")).unwrap();
        let history = std::fs::read(f.directory.join("chat_history.jsonl")).unwrap();
        let sampled = f.server.request_count();
        while f.gateway.try_recv().is_ok() {}
        let (observe, mut observed) = mpsc::unbounded_channel();
        let (actor, mut actors) = mpsc::unbounded_channel();
        let (commit, mut commits) = mpsc::unbounded_channel();
        let (sender, coordinator_owner) = coordinator(&f.agent, Some(ParentInputFault::PublishedReconfirm { observe, actor, commit }), None);
        let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
        let adoption = backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt);
        tokio::pin!(adoption);
        let (probe, actor) = bounded(async {
            tokio::select! {
                biased;
                pair = async { (observed.recv().await.unwrap(), actors.recv().await.unwrap()) } => pair,
                outcome = &mut adoption => panic!("adoption finished before the real reconfirm cut: {outcome:?}"),
            }
        }).await;
        let NativeAdoptionFaultProbe { owner, release, .. } = probe;
        flush_live_owner(&owner).await;
        match cut {
            FailedAdoption::ReconfirmTimeout => {
                assert_eq!(bounded(&mut adoption).await,
                    ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref: f.reference.clone() });
                drop(release);
            }
            FailedAdoption::LostReconfirmReply => {
                drop(release);
                assert_eq!(bounded(&mut adoption).await,
                    ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref: f.reference.clone() });
            }
            FailedAdoption::DeferredCommitRejected => {
                actor.cmd_tx.send(SessionCommand::Cancel(crate::session::CancelOptions {
                    trigger: Some(crate::session::CancelTrigger::Shutdown),
                    ..Default::default()
                })).unwrap();
                let (respond_to, copied) = oneshot::channel();
                actor.cmd_tx.send(SessionCommand::CopyFile { respond_to }).unwrap();
                bounded(copied).await.unwrap().unwrap();
                release.send(()).unwrap();
                let outcome = bounded(&mut adoption).await;
                assert!(matches!(bounded(commits.recv()).await, Some(crate::session::persistence::native_wake_tests_support::NativeDeferredCommit::Rejected)));
                assert_eq!(outcome,
                    ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref: f.reference.clone() });
            }
        }
        await_failed_attempt_teardown(&backend, &actor, &f.reference.receiver.session_id).await;
        flush_live_owner(&owner).await;
        let reopened = f.storage.load_summary(&f.info).await.unwrap();
        let retained = &reopened.completion_delivery.receiver.records[&f.reference.delivery_id];
        assert!(matches!(retained.execution, DeliveryExecution::Queued { .. }));
        assert_eq!(retained.parent_recovery_ref(), Some(f.reference.clone()));
        assert_eq!(serde_json::to_value(retained).unwrap(), original);
        assert_eq!(reopened.attempt_id, before.attempt_id);
        assert_eq!(reopened.next_trace_turn, before.next_trace_turn);
        assert_eq!(f.server.request_count(), sampled);
        assert_eq!(std::fs::read(f.artifacts.join("meta.json")).unwrap(), published_meta);
        assert_eq!(std::fs::read(f.artifacts.join("output.json")).unwrap(), f.prior_output);
        assert_eq!(std::fs::read(f.directory.join("chat_history.jsonl")).unwrap(), history);
        assert_eq!(std::fs::read(f.directory.join("child_management.json")).unwrap(), f.prior_recipe);
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Observe)).await,
            ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref: f.reference.clone(), state: MessageRecoveryState::Queued });
        flush_live_owner(&owner).await;
        drop(actor);
        drop(owner);
        bounded(async {
            loop {
                let (_, presence) = f.storage.observe_receiver_custody(&f.info).unwrap();
                match presence {
                    crate::session::receiver_execution::ReceiverPresence::Detached => break,
                    crate::session::receiver_execution::ReceiverPresence::Live => tokio::task::yield_now().await,
                }
            }
        }).await;
        let (reopen_lease, recovered) = f.storage.claim_receiver(&f.info).await.unwrap();
        assert_eq!(serde_json::to_value(&recovered.completion_delivery.receiver.records[&f.reference.delivery_id]).unwrap(), original);
        assert_eq!(recovered.attempt_id, before.attempt_id);
        assert_eq!(recovered.next_trace_turn, before.next_trace_turn);
        drop(reopen_lease);
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Observe)).await,
            ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref: f.reference.clone(), state: MessageRecoveryState::Queued });
        let mut response = f.server.expect_response_blocked("retained original native admission", InferenceRequestMatcher::foreground_containing(
            InferenceEndpoint::Responses, TEXT), ScriptedResponse::sse(
                xai_grok_test_support::sse::responses_api_script_exact(r#"{"value":"retained-original-result"}"#, "test-model")));
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await,
            ActiveAgentMessageOutcome::Accepted { message_id: f.reference.message_id.clone() });
        bounded(response.wait_blocked()).await;
        assert_eq!(std::fs::read(f.artifacts.join("meta.json")).unwrap(), published_meta);
        assert_eq!(std::fs::read(f.artifacts.join("output.json")).unwrap(), f.prior_output);
        let processing = f.storage.load_summary(&f.info).await.unwrap();
        assert!(matches!(processing.completion_delivery.receiver.records[&f.reference.delivery_id].execution, DeliveryExecution::Processing { .. }));
        assert_eq!(processing.attempt_id, before.attempt_id);
        assert_eq!(processing.next_trace_turn, before.next_trace_turn);
        let requests = f.server.request_bodies();
        let inputs = requests.iter().filter(|request| request.to_string().contains(TEXT)).collect::<Vec<_>>();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].to_string().matches(TEXT).count(), 1);
        assert_eq!(inputs[0].pointer("/text/format/schema"), Some(&original_schema()));
        assert_eq!(f.server.request_count(), sampled + 1);
        assert!(std::iter::from_fn(|| f.gateway.try_recv().ok()).all(|message| !matches!(message,
            xai_acp_lib::AcpClientMessage::ExtNotification(args) if args.request.params.get().contains("subagent_spawned"))));
        response.release();
        let result = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.expect("query acknowledgement").unwrap();
        assert!(matches!(result.status, SubagentSnapshotStatus::Completed { output, .. } if output.contains("retained-original-result")));
        response.assert_satisfied();
        let terminal = f.storage.load_summary(&f.info).await.unwrap();
        assert_eq!(terminal.attempt_id, before.attempt_id);
        assert_eq!(terminal.next_trace_turn, before.next_trace_turn);
        assert!(matches!(terminal.completion_delivery.receiver.records[&f.reference.delivery_id].execution, DeliveryExecution::Succeeded));
        let history = std::fs::read_to_string(f.directory.join("chat_history.jsonl")).unwrap();
        let bindings = history.lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap().get("_session_delivery").cloned())
            .filter(|binding| binding["delivery_id"] == f.reference.delivery_id).collect::<Vec<_>>();
        assert_eq!(bindings.len(), 1);
        let binding: xai_chat_state::DeliveryBinding = serde_json::from_value(bindings[0].clone()).unwrap();
        assert_eq!(binding.turn_id, f.reference.prompt_id);
        assert_eq!(binding.payload_digest, f.reference.payload_digest);
        assert!(matches!(binding.parent_task, xai_chat_state::ParentTask::Instance { id } if !id.is_empty()));
        drop(coordinator_owner);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_live_reconfirm_timeout_and_actor_drop_retain_original_queued_admission() {
    failed_adopter_retains_original(FailedAdoption::ReconfirmTimeout).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_live_lost_reconfirm_reply_and_actor_drop_retain_original_queued_admission() {
    failed_adopter_retains_original(FailedAdoption::LostReconfirmReply).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_rejected_real_deferred_commit_retains_original_queued_admission() {
    failed_adopter_retains_original(FailedAdoption::DeferredCommitRejected).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_released_adoption_gate_teardown_before_projection_retains_original() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let mut f = lost_owner(ParentInputFault::ShutdownBeforeProjection).await;
        let before = f.storage.load_summary(&f.info).await.unwrap();
        let record = serde_json::to_value(&before.completion_delivery.receiver.records[&f.reference.delivery_id]).unwrap();
        let meta = std::fs::read(f.artifacts.join("meta.json")).unwrap();
        let sampled = f.server.request_count();
        while f.gateway.try_recv().is_ok() {}
        let (observe, mut observed) = mpsc::unbounded_channel();
        let (actor, mut actors) = mpsc::unbounded_channel();
        let (sender, coordinator_owner) = coordinator(&f.agent, Some(ParentInputFault::BeforeAdoptedProjection { observe, actor }), None);
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
        await_failed_attempt_teardown(&backend, &actor, &f.reference.receiver.session_id).await;
        flush_live_owner(&probe.owner).await;
        let retained = f.storage.load_summary(&f.info).await.unwrap();
        assert_eq!(serde_json::to_value(&retained.completion_delivery.receiver.records[&f.reference.delivery_id]).unwrap(), record);
        assert_eq!(retained.attempt_id, before.attempt_id);
        assert_eq!(retained.next_trace_turn, before.next_trace_turn);
        assert_eq!(std::fs::read(f.artifacts.join("meta.json")).unwrap(), meta);
        assert_eq!(std::fs::read(f.artifacts.join("output.json")).unwrap(), f.prior_output);
        assert_eq!(std::fs::read(f.directory.join("chat_history.jsonl")).unwrap(), f.prior_history);
        assert_eq!(f.server.request_count(), sampled);
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Observe)).await,
            ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref: f.reference.clone(), state: MessageRecoveryState::Queued });
        drop(actor);
        drop(probe.owner);
        bounded(async {
            loop {
                let (_, presence) = f.storage.observe_receiver_custody(&f.info).unwrap();
                match presence {
                    crate::session::receiver_execution::ReceiverPresence::Detached => break,
                    crate::session::receiver_execution::ReceiverPresence::Live => tokio::task::yield_now().await,
                }
            }
        }).await;
        let response = f.server.expect_response("retry released native gate", InferenceRequestMatcher::foreground_containing(
            InferenceEndpoint::Responses, TEXT), ScriptedResponse::sse(
                xai_grok_test_support::sse::responses_api_script_exact(r#"{"value":"released-gate-retry-result"}"#, "test-model")));
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await,
            ActiveAgentMessageOutcome::Accepted { message_id: f.reference.message_id.clone() });
        let result = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.expect("query acknowledgement").unwrap();
        assert!(matches!(result.status, SubagentSnapshotStatus::Completed { output, .. } if output.contains("released-gate-retry-result")));
        response.assert_satisfied();
        let completed = f.storage.load_summary(&f.info).await.unwrap();
        assert!(matches!(completed.completion_delivery.receiver.records[&f.reference.delivery_id].execution, DeliveryExecution::Succeeded));
        assert_eq!(completed.next_trace_turn, before.next_trace_turn);
        assert_eq!(f.server.request_count(), sampled + 1);
        let history = std::fs::read_to_string(f.directory.join("chat_history.jsonl")).unwrap();
        assert_eq!(history.lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap().get("_session_delivery").cloned())
            .filter(|binding| binding["delivery_id"] == f.reference.delivery_id).count(), 1);
        assert!(std::iter::from_fn(|| f.gateway.try_recv().ok()).all(|message| !matches!(message,
            xai_acp_lib::AcpClientMessage::ExtNotification(args) if args.request.params.get().contains("subagent_spawned"))));
        drop(coordinator_owner);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_reserved_adoption_explicit_cancel_authorizes_original_terminal() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner(ParentInputFault::ShutdownBeforeProjection).await;
        let (observe, mut observed) = mpsc::unbounded_channel();
        let (actor, mut actors) = mpsc::unbounded_channel();
        let (commit, mut commits) = mpsc::unbounded_channel();
        let (sender, coordinator_owner) = coordinator(&f.agent, Some(ParentInputFault::PublishedReconfirm { observe, actor, commit }), None);
        let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
        let adoption = backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt);
        tokio::pin!(adoption);
        let (probe, actor) = bounded(async {
            tokio::select! {
                biased;
                pair = async { (observed.recv().await.unwrap(), actors.recv().await.unwrap()) } => pair,
                outcome = &mut adoption => panic!("adoption finished before explicit cancellation: {outcome:?}"),
            }
        }).await;
        let sampled = f.server.request_count();
        actor.cmd_tx.send(SessionCommand::Cancel(crate::session::CancelOptions {
            trigger: Some(crate::session::CancelTrigger::Esc),
            ..Default::default()
        })).unwrap();
        let (respond_to, copied) = oneshot::channel();
        actor.cmd_tx.send(SessionCommand::CopyFile { respond_to }).unwrap();
        bounded(copied).await.unwrap().unwrap();
        probe.release.send(()).unwrap();
        assert_eq!(bounded(&mut adoption).await,
            ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref: f.reference.clone() });
        assert!(matches!(bounded(commits.recv()).await, Some(crate::session::persistence::native_wake_tests_support::NativeDeferredCommit::Rejected)));
        await_failed_attempt_teardown(&backend, &actor, &f.reference.receiver.session_id).await;
        flush_live_owner(&probe.owner).await;
        let terminal = f.storage.load_summary(&f.info).await.unwrap();
        assert!(matches!(terminal.completion_delivery.receiver.records[&f.reference.delivery_id].execution, DeliveryExecution::Cancelled { .. }));
        assert_eq!(f.server.request_count(), sampled);
        assert_eq!(std::fs::read(f.directory.join("chat_history.jsonl")).unwrap(), f.prior_history);
        drop(actor);
        drop(probe.owner);
        drop(coordinator_owner);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_published_adoption_processing_explicit_cancel_is_terminal() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new()
        .run_until(async {
            let f = lost_owner(ParentInputFault::ShutdownBeforeProjection).await;
            let (sender, owner) = coordinator(&f.agent, None, None);
            let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
            let mut response = f.server.expect_response_blocked(
                "cancel real adopted processing",
                InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Responses, TEXT),
                ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact(
                    r#"{"value":"cancelled-native-result"}"#,
                    "test-model",
                )),
            );
            assert_eq!(
                bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt))
                    .await,
                ActiveAgentMessageOutcome::Accepted {
                    message_id: f.reference.message_id.clone()
                }
            );
            bounded(response.wait_blocked()).await;
            let processing = f.storage.load_summary(&f.info).await.unwrap();
            assert!(matches!(
                processing.completion_delivery.receiver.records[&f.reference.delivery_id].execution,
                DeliveryExecution::Processing { .. }
            ));
            let live = bounded(f.agent.management_session(&f.info.id))
                .await
                .unwrap();
            live.handle
                .cmd_tx
                .send(SessionCommand::Cancel(crate::session::CancelOptions {
                    trigger: Some(crate::session::CancelTrigger::Esc),
                    ..Default::default()
                }))
                .unwrap();
            drop(live);
            let result =
                bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None))
                    .await.expect("query acknowledgement")
                    .unwrap();
            assert!(matches!(
                result.status,
                SubagentSnapshotStatus::Cancelled { .. }
            ));
            response.release();
            response.assert_satisfied();
            let cancelled = f.storage.load_summary(&f.info).await.unwrap();
            assert!(matches!(
                cancelled.completion_delivery.receiver.records[&f.reference.delivery_id].execution,
                DeliveryExecution::Cancelled { .. }
            ));
            assert_eq!(
                bounded(
                    backend.recover_message(f.reference.clone(), MessageRecoveryAction::Observe)
                )
                .await,
                ActiveAgentMessageOutcome::RecoveryObserved {
                    recovery_ref: f.reference.clone(),
                    state: MessageRecoveryState::Cancelled
                }
            );
            drop(owner);
        })
        .await;
}
