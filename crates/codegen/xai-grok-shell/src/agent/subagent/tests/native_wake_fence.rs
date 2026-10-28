use super::*;
use crate::session::delivery_receiver::{DeliveryExecution, DeliveryProvenance};
use crate::session::persistence::native_wake_tests_support::ParentInputFault;
use crate::session::storage::{AtomicWriteFault, StorageAdapter};
use xai_grok_tools::implementations::grok_build::task::backend::{ChannelBackend, SubagentBackend};
use xai_grok_tools::implementations::grok_build::task::coordinator::{
    CoordinatorConfig, SubagentCoordinator,
};

#[tokio::test(flavor = "current_thread")]
async fn native_wake_admission_write_fault_preserves_completed_result() {
    fault_preserves_completed(ParentInputFault::AtomicWrite(AtomicWriteFault::Write)).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_wake_admission_sync_fault_preserves_completed_result() {
    fault_preserves_completed(ParentInputFault::AtomicWrite(AtomicWriteFault::Sync)).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_wake_admission_parent_sync_fault_preserves_completed_result() {
    fault_preserves_completed(ParentInputFault::AtomicWrite(AtomicWriteFault::ParentSync)).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_wake_lost_admission_ack_cannot_publish_or_start() {
    fault_preserves_completed(ParentInputFault::LoseAdmissionAck).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_wake_lost_reservation_ack_cannot_publish_or_start() {
    fault_preserves_completed(ParentInputFault::LoseReservationAck).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_wake_conflicting_reservation_cannot_publish_or_start() {
    fault_preserves_completed(ParentInputFault::ReservationDigestConflict).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_wake_persistence_shutdown_before_handoff_preserves_completed_result() {
    fault_preserves_completed(ParentInputFault::ShutdownAfterAdmission).await;
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(45), future)
        .await
        .unwrap()
}

async fn fault_preserves_completed(fault: ParentInputFault) {
    tokio::task::LocalSet::new()
        .run_until(async {
            let meta_dir = tempfile::tempdir().unwrap();
            let server = xai_grok_test_support::MockInferenceServer::start()
                .await
                .unwrap();
            server.set_response("native completed result before the admission cut");
            let id = uuid::Uuid::now_v7().to_string();
            let mut ordinary_ctx = ctx_with_toggle(HashMap::new());
            configure_completion_harness(
                &mut ordinary_ctx,
                &server,
                RunShellChildHarnessConfig::new(
                    meta_dir.path().to_path_buf(),
                    InitialAttemptBehavior::Normal,
                ),
            );
            let child_info = SessionInfo {
                id: acp::SessionId::new(id.clone()),
                cwd: ordinary_ctx.parent_cwd.to_string_lossy().into_owned(),
            };
            let (parent_cmd_tx, parent_cmd_rx) = mpsc::unbounded_channel();
            ordinary_ctx.parent_cmd_tx = Some(parent_cmd_tx.clone());
            let usage_ack = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(
                acknowledge_parent_usage(parent_cmd_rx),
            ));
            let mut wake_ctx = ctx_with_toggle(HashMap::new());
            let mut harness = RunShellChildHarnessConfig::new(
                meta_dir.path().to_path_buf(),
                InitialAttemptBehavior::Normal,
            );
            harness.parent_input_fault = Some(fault);
            configure_completion_harness(&mut wake_ctx, &server, harness);
            wake_ctx.parent_cmd_tx = Some(parent_cmd_tx);
            let (gateway, mut gateway_rx) = test_gateway_with_receiver();
            let (command_tx, command_rx) =
                SubagentCoordinator::<RunShellChildTestRunner>::channel();
            let coordinator = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(
                SubagentCoordinator::from_channel(
                    command_rx,
                    RunShellChildTestRunner::new([ordinary_ctx, wake_ctx], false, gateway),
                    CoordinatorConfig::default(),
                )
                .run(),
            ));
            let backend = ChannelBackend::for_coordinator_session(command_tx, "setup-parent");
            let prior = bounded(backend.spawn(auto_wake_test_request(&id), None, None))
                .await
                .unwrap();
            assert!(
                prior.success(),
                "ordinary native worker must actually complete: {prior:?}"
            );
            assert!(
                prior
                    .output
                    .contains("native completed result before the admission cut")
            );
            let session_dir = crate::session::persistence::session_dir(&child_info);
            let prior_history = std::fs::read(session_dir.join("chat_history.jsonl")).unwrap();
            let prior_meta = std::fs::read(meta_dir.path().join("meta.json")).unwrap();
            assert!(write_subagent_output(
                meta_dir.path(),
                prior.output.as_ref()
            ));
            let prior_output = std::fs::read(meta_dir.path().join("output.json")).unwrap();
            let storage = crate::session::storage::JsonlStorageAdapter::with_root(
                crate::util::grok_home::grok_home(),
            );
            let prior_summary = storage.load_summary(&child_info).await.unwrap();
            let requests_before = server.request_count();
            while gateway_rx.try_recv().is_ok() {}
            let outcome = bounded(
                backend.send_active_message(
                    ActiveAgentMessageRequest::try_new(&id, "literal native wake at a fault cut")
                        .unwrap(),
                ),
            )
            .await;
            let ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref } = outcome else {
                panic!("unconfirmed native admission: {outcome:?}");
            };
            assert_eq!(recovery_ref.receiver.session_id, id);
            assert_eq!(
                recovery_ref.payload_digest,
                "daac322604fcb24c23cfbb43d63f82fedf01cc551dd08457f67a7dc0162d20b0"
            );
            let restored = bounded(backend.query(&id, true, Some(30_000), None))
                .await
                .unwrap();
            assert!(
                matches!(&restored.status, SubagentSnapshotStatus::Completed { output, .. }
            if output.as_str() == prior.output.as_ref())
            );
            assert_eq!(
                server.request_count(),
                requests_before,
                "no model request may cross an unconfirmed fence"
            );
            assert_eq!(
                std::fs::read(session_dir.join("chat_history.jsonl")).unwrap(),
                prior_history
            );
            assert_eq!(
                std::fs::read(meta_dir.path().join("meta.json")).unwrap(),
                prior_meta
            );
            assert_eq!(
                std::fs::read(meta_dir.path().join("output.json")).unwrap(),
                prior_output
            );
            let (lease, summary) = storage
                .claim_receiver(&child_info)
                .await
                .expect("failed native reservation must release its exact execution lease");
            assert_eq!(summary.attempt_id, prior_summary.attempt_id);
            assert_eq!(summary.next_trace_turn, prior_summary.next_trace_turn);
            for (id, prior) in &prior_summary.completion_delivery.receiver.records {
                assert_eq!(
                    serde_json::to_value(summary.completion_delivery.receiver.records.get(id))
                        .unwrap(),
                    serde_json::to_value(Some(prior)).unwrap()
                );
            }
            let mut new_admissions = 0;
            for (id, record) in &summary.completion_delivery.receiver.records {
                if prior_summary
                    .completion_delivery
                    .receiver
                    .records
                    .contains_key(id)
                {
                    continue;
                }
                new_admissions += 1;
                let DeliveryProvenance::ParentMessage { admission } = &record.provenance else {
                    panic!("native cut lost authenticated parent provenance");
                };
                assert_eq!(
                    record.digest,
                    "daac322604fcb24c23cfbb43d63f82fedf01cc551dd08457f67a7dc0162d20b0"
                );
                assert_eq!(admission.sender_session_id, "setup-parent");
                assert_eq!(recovery_ref.message_id, admission.message_id);
                assert_eq!(recovery_ref.prompt_id, admission.prompt_id);
                assert_eq!(admission.text, "literal native wake at a fault cut");
                assert_eq!(
                    admission.prompt_id,
                    format!("parent-message-{}", admission.message_id)
                );
                assert!(matches!(
                    &record.execution,
                    DeliveryExecution::Queued { .. } | DeliveryExecution::Cancelled { .. }
                ));
            }
            assert!(
                new_admissions <= 1,
                "a failed bootstrap must not mint a second identity"
            );
            drop(lease);
            assert!(
                std::iter::from_fn(|| gateway_rx.try_recv().ok()).all(|message| {
                    !matches!(message, xai_acp_lib::AcpClientMessage::ExtNotification(args)
                if args.request.params.get().contains("subagent_spawned"))
                }),
                "a failed reservation cannot publish a new native lifecycle"
            );
            drop(backend);
            drop(coordinator);
            drop(usage_ack);
        })
        .await;
}
