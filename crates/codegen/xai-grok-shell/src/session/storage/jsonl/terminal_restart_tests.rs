use super::*;
use crate::session::completion_delivery::{CompletionPayloadSource, CompletionEventPolicy};
use crate::session::delivery_receiver::ReceiverTurnOrigin;
use xai_chat_state::completion::{CompletionIdentity, CompletionTaskKind, TerminalCompletionId};

#[tokio::test]
async fn typed_goal_suppression_survives_reenrollment_restart_without_consuming_automatic_peers() {
    use crate::session::completion_delivery::{CompletionObligation, CompletionSuppression};
    use xai_grok_tools::computer::types::TaskKind;
    let (directory, storage, info) = fixture().await;
    let suppressed: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "stable-execution".into(), task_kind: CompletionTaskKind::Bash }.into();
    storage.enroll_terminal_completion(&info, snapshot(&info.id.to_string(), TaskKind::Bash), "goal result".into(), CompletionEventPolicy::Automatic).await.unwrap();
    storage.suppress_terminal_completions(&info, vec![suppressed.clone()], CompletionSuppression::GoalNotificationPolicy).await.unwrap();
    storage.enroll_terminal_completion(&info, snapshot(&info.id.to_string(), TaskKind::Bash), "retry must not rearm".into(), CompletionEventPolicy::Automatic).await.unwrap();
    storage.enroll_terminal_completion(&info, snapshot(&info.id.to_string(), TaskKind::Monitor), "automatic peer".into(), CompletionEventPolicy::Automatic).await.unwrap();
    let restored = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    let state = restored.recover_subagent_completions(&info).await.unwrap().completion_delivery;
    assert!(state.receipts.is_empty(), "policy suppression must never manufacture a model-history receipt");
    assert!(state.obligations.iter().any(|entry| matches!(entry, CompletionObligation::Suppressed { id, reason: CompletionSuppression::GoalNotificationPolicy, .. } if id == &suppressed)));
    assert!(state.obligations.iter().any(|entry| matches!(entry, CompletionObligation::Pending { payload, .. } if payload.text == "automatic peer" && matches!(payload.policy, CompletionEventPolicy::Automatic))));
    let caller = recovery::task("stale-notification-caller");
    assert!(matches!(restored.append_completion_commit_aware(&info, &projection(&[suppressed], &caller), &caller).await, Err(StrictAppendError::NotCommitted(error)) if error.kind() == io::ErrorKind::PermissionDenied));
    assert!(restored.read_chat_history_sync(restored.chat_file(&info), CHAT_FORMAT_VERSION).unwrap().is_empty());
}

#[tokio::test]
async fn explicit_tool_observation_of_a_suppressed_task_retains_caller_recovery() {
    use crate::session::completion_delivery::{CompletionObligation, CompletionSuppression};
    use xai_grok_tools::computer::types::TaskKind;
    let (directory, storage, info) = fixture().await;
    let id: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "stable-execution".into(), task_kind: CompletionTaskKind::Bash }.into();
    storage.enroll_terminal_completion(&info, snapshot(&info.id.to_string(), TaskKind::Bash), "explicitly observed result".into(), CompletionEventPolicy::Automatic).await.unwrap();
    storage.suppress_terminal_completions(&info, vec![id.clone()], CompletionSuppression::GoalNotificationPolicy).await.unwrap();
    let owner = recovery::task("explicit-observer");
    let mut observed = projection(&[id.clone()], &owner);
    observed.item = ConversationItem::tool_result("explicit-terminal-read", "actual tool result");
    assert!(matches!(storage.append_completion_commit_aware(&info, &observed, &owner).await.unwrap(), CompletionProjectionAck::Appended));
    let restored = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    let state = restored.recover_subagent_completions(&info).await.unwrap().completion_delivery;
    assert!(state.obligations.iter().any(|obligation| matches!(obligation, CompletionObligation::Projected { receipt, payload } if receipt.completion == id && receipt.parent_task == owner.parent_task && payload.text == "explicitly observed result")));
    assert!(matches!(restored.append_completion_commit_aware(&info, &observed, &owner).await.unwrap(), CompletionProjectionAck::AlreadyPresent { .. }));
}

#[tokio::test]
async fn goal_suppression_cannot_erase_a_committed_but_unindexed_caller_projection() {
    use crate::session::completion_delivery::{CompletionObligation, CompletionSuppression};
    use xai_grok_tools::computer::types::TaskKind;
    let (_directory, storage, info) = fixture().await;
    let id: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "stable-execution".into(), task_kind: CompletionTaskKind::Bash }.into();
    storage.enroll_terminal_completion(&info, snapshot(&info.id.to_string(), TaskKind::Bash), "observed".into(), CompletionEventPolicy::Automatic).await.unwrap();
    let owner = recovery::task("actual-caller");
    assert!(matches!(storage.append_completion_sync(&info, &projection(&[id.clone()], &owner), &owner, |_| Err(io::Error::other("lost commit ACK"))), Err(StrictAppendError::Committed { .. })));
    storage.suppress_terminal_completions(&info, vec![id.clone()], CompletionSuppression::GoalNotificationPolicy).await.unwrap();
    let state = storage.read_completion_state(&info).await.unwrap();
    assert!(state.obligations.iter().any(|entry| matches!(entry, CompletionObligation::Projected { receipt, .. } if receipt.completion == id && receipt.parent_task == owner.parent_task)));
    assert_eq!(state.receipts.len(), 1);
}

fn workflow(revision: u64) -> CompletionPayloadSource {
    CompletionPayloadSource::Workflow(Box::new(serde_json::from_value(serde_json::json!({
        "run_id": "restart-run", "revision": revision, "name": "durability",
        "objective": "retain terminal revision", "status": "budget_limited"
    })).unwrap()))
}

fn snapshot(owner: &str, kind: xai_grok_tools::computer::types::TaskKind) -> CompletionPayloadSource {
    use xai_grok_tools::computer::types::TaskSnapshot;
    CompletionPayloadSource::Task(Box::new(TaskSnapshot {
        task_id: "stable-execution".into(), command: "finished task".into(), display_command: None,
        cwd: "/tmp".into(), start_time: std::time::SystemTime::UNIX_EPOCH,
        end_time: Some(std::time::SystemTime::UNIX_EPOCH), output: "terminal evidence".into(),
        output_file: Default::default(), truncated: false, exit_code: Some(0), signal: None,
        completed: true, kind, block_waited: false, explicitly_killed: false,
        kill_result_delivered: false, owner_session_id: Some(owner.into()), description: None,
        is_backgrounded: true, output_total_bytes: 17,
    }))
}

fn projection(ids: &[CompletionIdentity], owner: &xai_chat_state::InputAdmission) -> CompletionProjection {
    let mut projection = CompletionProjection::new(ids[0].clone(), "reused-parent-prompt".into(), "one terminal batch".into());
    projection.receipt.parent_task = owner.parent_task.clone();
    projection.additional_receipts = ids[1..].iter().map(|id| CompletionReceipt {
        completion: id.clone(), prompt_id: "reused-parent-prompt".into(), parent_task: owner.parent_task.clone(),
    }).collect();
    projection
}

#[tokio::test]
async fn mixed_v1_v2_batch_survives_lost_ack_restart_and_old_parent_terminal() {
    use xai_grok_tools::computer::types::TaskKind;
    let (directory, storage, info) = fixture().await;
    let child = event("real-child", xai_message_delivery_core::AttemptId::mint(1), SubagentState::Completed);
    storage.prepare_subagent_completion(&info, child.clone()).await.unwrap();
    storage.publish_subagent_completion(&info, &child, true).await.unwrap();
    for kind in [TaskKind::Bash, TaskKind::Monitor] {
        storage.enroll_terminal_completion(&info, snapshot(&info.id.to_string(), kind), "terminal evidence".into(), CompletionEventPolicy::Automatic).await.unwrap();
    }
    for revision in [7, 8] {
        storage.enroll_terminal_completion(&info, workflow(revision), "budget exhausted".into(), CompletionEventPolicy::Automatic).await.unwrap();
    }
    let ids: Vec<CompletionIdentity> = vec![
        child.id.clone().into(),
        TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "stable-execution".into(), task_kind: CompletionTaskKind::Bash }.into(),
        TerminalCompletionId::Task { owner_session_id: info.id.to_string(), task_id: "stable-execution".into(), task_kind: CompletionTaskKind::Monitor }.into(),
        TerminalCompletionId::Workflow { run_id: "restart-run".into(), revision: 7 }.into(),
        TerminalCompletionId::Workflow { run_id: "restart-run".into(), revision: 8 }.into(),
    ];
    let old = recovery::task("old-terminal-parent");
    assert!(matches!(storage.append_completion_sync(&info, &projection(&ids, &old), &old,
        |_| Err(io::Error::other("withheld durable acknowledgement"))), Err(StrictAppendError::Committed { .. })));
    let restored = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    let state = restored.recover_subagent_completions(&info).await.unwrap().completion_delivery;
    assert_eq!(state.obligations.len(), 5);
    assert_eq!(state.receipts.len(), 5);
    let new = recovery::task("replacement-terminal-parent");
    let CompletionProjectionAck::Continue { history, .. } = restored.append_completion_commit_aware(&info, &projection(&ids, &new), &new).await.unwrap() else { panic!("unfinished terminal batch must continue its existing history") };
    assert_eq!(history.iter().filter(|item| item.text_content() == "one terminal batch").count(), 1);
    restored.append_update_durable_commit_aware(&info, &recovery::terminal(&info, "reused-parent-prompt", &old, ReceiverTurnOrigin::Other)).await.unwrap();
    assert_eq!(restored.recover_subagent_completions(&info).await.unwrap().completion_delivery.obligations.len(), 5);
    restored.append_update_durable_commit_aware(&info, &recovery::terminal(&info, "reused-parent-prompt", &new, ReceiverTurnOrigin::Other)).await.unwrap();
    let state = restored.recover_subagent_completions(&info).await.unwrap().completion_delivery;
    assert!(state.obligations.is_empty());
    assert_eq!(state.receipts.len(), 5);
    assert!(matches!(restored.append_completion_commit_aware(&info, &projection(&ids, &old), &old).await.unwrap(), CompletionProjectionAck::AlreadyPresent { .. }));
    assert_eq!(restored.read_chat_history_sync(restored.chat_file(&info), CHAT_FORMAT_VERSION).unwrap().iter().filter(|item| item.text_content() == "one terminal batch").count(), 1);
}

#[tokio::test]
async fn cancelled_admission_cannot_consume_enrolled_terminal_revisions() {
    let (directory, storage, info) = fixture().await;
    storage.enroll_terminal_completion(&info, workflow(3), "budget exhausted".into(), CompletionEventPolicy::Automatic).await.unwrap();
    let ids = vec![TerminalCompletionId::Workflow { run_id: "restart-run".into(), revision: 3 }.into()];
    let closed = recovery::task("cancelled-parent");
    closed.cancellation.cancel();
    assert!(matches!(storage.append_completion_commit_aware(&info, &projection(&ids, &closed), &closed).await.unwrap(), CompletionProjectionAck::TurnClosed));
    let restored = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    let state = restored.recover_subagent_completions(&info).await.unwrap().completion_delivery;
    assert!(state.receipts.is_empty());
    assert_eq!(state.obligations.len(), 1);
    let replacement = recovery::task("replacement-parent");
    assert!(matches!(restored.append_completion_commit_aware(&info, &projection(&ids, &replacement), &replacement).await.unwrap(), CompletionProjectionAck::Appended));
}

#[tokio::test]
async fn parent_admission_is_durable_without_model_visibility_and_rejects_conflicting_source() {
    use crate::session::delivery_receiver::{ParentMessageAdmission, ParentMessageOperation, ParentMessageSource, DeliveryExecution, DeliveryProvenance};
    use crate::session::receiver_execution::ReceiverAccess;
    let (directory, storage, info) = fixture().await;
    let (lease, _) = storage.claim_receiver(&info).await.unwrap();
    let message = ParentMessageAdmission {
        message_id: "steer-id".into(), sender_session_id: "owning-parent".into(), source: ParentMessageSource::Human,
        effective: ParentMessageOperation::Steer { prompt_id: "active-tool-turn".into(), parent_task: recovery::task("live-instance").parent_task },
        prompt_id: "parent-message-steer-id".into(), text: "held until tool completes".into(),
    };
    let ack = storage.admit_parent_message(&info, &message, crate::session::delivery_receiver::ParentAdmissionAuthority::DirectLocal, ReceiverAccess::Owner(&lease)).await.unwrap();
    assert!(matches!(ack, crate::session::delivery_receiver::ParentAdmissionAck::DirectLocal));
    let restored = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    let state = restored.read_completion_state(&info).await.unwrap();
    let record = state.receiver.records.get(&message.delivery_id()).unwrap();
    assert!(matches!(&record.execution, DeliveryExecution::Queued { payload, .. } if payload == &message.text));
    assert!(matches!(&record.provenance, DeliveryProvenance::ParentMessage { admission } if admission == &message));
    assert!(restored.read_chat_history_sync(restored.chat_file(&info), CHAT_FORMAT_VERSION).unwrap().iter().all(|item| item.text_content() != message.text));
    let mut conflict = message.clone();
    conflict.source = ParentMessageSource::Agent;
    assert_eq!(storage.admit_parent_message(&info, &conflict, crate::session::delivery_receiver::ParentAdmissionAuthority::DirectLocal, ReceiverAccess::Owner(&lease)).await.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    let owner = recovery::task("live-instance");
    let projection = xai_chat_state::DeliveryProjection {
        item: ConversationItem::parent_human_message(message.text.clone()),
        binding: xai_chat_state::DeliveryBinding { delivery_id: message.delivery_id(), payload_digest: crate::session::delivery_receiver::payload_digest(&message.text), turn_id: "active-tool-turn".into(), parent_task: owner.parent_task.clone() },
    };
    assert!(matches!(storage.append_delivery_commit_aware(&info, &projection, &owner).await.unwrap(), CompletionProjectionAck::Appended));
    crate::session::storage::summary_write::mutate_locked(&storage.summary_file(&info), &storage.summary_lock_file(&info), |summary| {
        let record = summary.completion_delivery.receiver.records.get_mut(&message.delivery_id()).unwrap();
        record.execution = DeliveryExecution::Queued { payload: message.text.clone(), prompt_id: message.prompt_id.clone() };
        record.revision = 1;
        Ok(())
    }).unwrap();
    drop(lease);
    let (_replacement_lease, recovered) = restored.claim_receiver(&info).await.unwrap();
    assert!(matches!(&recovered.completion_delivery.receiver.records[&message.delivery_id()].execution, DeliveryExecution::Interrupted { .. }), "history receipt must repair a crash before the derived processing cache, never replay the input");
    assert!(matches!(restored.append_delivery_commit_aware(&info, &projection, &owner).await.unwrap(), CompletionProjectionAck::AlreadyPresent { .. }));
    assert_eq!(restored.read_chat_history_sync(restored.chat_file(&info), CHAT_FORMAT_VERSION).unwrap().iter().filter(|item| item.text_content() == message.text).count(), 1);
}
