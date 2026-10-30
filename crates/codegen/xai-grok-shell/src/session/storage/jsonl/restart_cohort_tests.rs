use super::*;
use crate::session::delivery_receiver::{DeliveryAction, DeliveryExecution, DeliveryRequest};
use crate::session::receiver_execution::ReceiverAccess;
use xai_grok_tools::types::restart_recovery::{RestartAdmission, RestartEpoch};

fn stale_admission() -> RestartAdmission {
    RestartAdmission::Quiescent { epoch: RestartEpoch(uuid::Uuid::now_v7()) }
}

#[tokio::test]
async fn consumed_epoch_cancels_old_receivers_without_touching_new_deliveries() {
    let (_directory, storage, info) = fixture().await;
    let (lease, _) = storage.claim_receiver(&info).await.unwrap();
    let request = |id: &str| DeliveryRequest {
        session_id: info.id.to_string(), delivery_id: id.into(), payload: id.into(), action: DeliveryAction::Admit,
    };
    storage.receive_delivery(&info, &request("old"), ReceiverAccess::Owner(&lease)).await.unwrap();
    let admission = stale_admission();
    storage.set_restart_admission(&info, admission, ReceiverAccess::Owner(&lease)).await.unwrap();
    let stopped = storage.read_completion_state(&info).await.unwrap();
    assert!(matches!(stopped.receiver.records.get("old").unwrap().execution, DeliveryExecution::Cancelled { .. }));
    storage.set_restart_admission(&info, admission.after_user_message(), ReceiverAccess::Owner(&lease)).await.unwrap();
    storage.receive_delivery(&info, &request("new"), ReceiverAccess::Owner(&lease)).await.unwrap();
    drop(lease);
    let (lease, restored) = storage.claim_receiver(&info).await.unwrap();
    assert!(restored.completion_delivery.restart_admission.permits_recovery());
    assert!(matches!(restored.completion_delivery.receiver.records.get("new").unwrap().execution, DeliveryExecution::Queued { .. }));
    assert!(matches!(restored.completion_delivery.receiver.records.get("old").unwrap().execution, DeliveryExecution::Cancelled { .. }));
    storage.set_restart_admission(&info, admission.after_user_message(), ReceiverAccess::Owner(&lease)).await.unwrap();
    let state = storage.read_completion_state(&info).await.unwrap();
    assert!(matches!(state.receiver.records.get("new").unwrap().execution, DeliveryExecution::Queued { .. }));
}

#[tokio::test]
async fn stale_suppression_retains_receipts_without_rearming_on_cold_recovery() {
    let (_directory, storage, info) = fixture().await;
    let event = event("old-child", xai_message_delivery_core::AttemptId::mint(1), SubagentState::Completed);
    storage.prepare_subagent_completion(&info, event.clone()).await.unwrap();
    storage.publish_subagent_completion(&info, &event, true).await.unwrap();
    storage.append_completion_commit_aware(&info,
        &CompletionProjection::new(event.id.clone(), "old-turn".into(), event.text.clone()), &admission(),
    ).await.unwrap();
    let (lease, _) = storage.claim_receiver(&info).await.unwrap();
    storage.set_restart_admission(&info, stale_admission(), ReceiverAccess::Owner(&lease)).await.unwrap();
    let state = storage.read_completion_state(&info).await.unwrap();
    assert_eq!(state.receipts.len(), 1);
    assert!(state.obligations.iter().all(|entry| matches!(entry, CompletionObligation::Suppressed { .. })));
    drop(lease);
    let (_lease, _) = storage.claim_receiver(&info).await.unwrap();
    storage.recover_subagent_completions(&info).await.unwrap();
    let state = storage.read_completion_state(&info).await.unwrap();
    assert_eq!(state.receipts.len(), 1);
    assert!(state.obligations.iter().all(|entry| matches!(entry, CompletionObligation::Suppressed { .. })));
}

#[tokio::test]
async fn failed_epoch_write_does_not_claim_durable_cancellation() {
    let (_directory, storage, info) = fixture().await;
    let (lease, _) = storage.claim_receiver(&info).await.unwrap();
    let lock = storage.summary_lock_file(&info);
    let backup = lock.with_extension("saved-lock");
    std::fs::rename(&lock, &backup).unwrap();
    std::fs::create_dir(&lock).unwrap();
    assert!(storage.set_restart_admission(&info, stale_admission(), ReceiverAccess::Owner(&lease)).await.is_err());
    std::fs::remove_dir(&lock).unwrap();
    std::fs::rename(&backup, &lock).unwrap();
    assert_eq!(storage.read_completion_state(&info).await.unwrap().restart_admission, RestartAdmission::Unaffected);
    storage.set_restart_admission(&info, stale_admission(), ReceiverAccess::Owner(&lease)).await.unwrap();
    assert!(!storage.read_completion_state(&info).await.unwrap().restart_admission.permits_recovery());
}
