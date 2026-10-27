use super::*;
use crate::session::delivery_receiver::{
    DeliveryAction, DeliveryProgress, DeliveryRequest, DeliveryResult, ReceiverTurnOrigin,
};
use crate::session::receiver_execution::ReceiverAccess;
use crate::session::storage::SessionUpdate;
use xai_chat_state::{DeliveryBinding, DeliveryProjection, InputAdmission, ParentTask};

pub(super) fn task(id: &str) -> InputAdmission {
    InputAdmission {
        parent_task: ParentTask::Instance { id: id.into() },
        ..admission()
    }
}

fn projected(
    event: &SubagentCompletionEvent,
    prompt: &str,
    admission: &InputAdmission,
) -> CompletionProjection {
    let mut projection =
        CompletionProjection::new(event.id.clone(), prompt.into(), event.text.clone());
    projection.receipt.parent_task = admission.parent_task.clone();
    projection
}

pub(super) fn terminal(
    info: &Info,
    prompt: &str,
    admission: &InputAdmission,
    origin: ReceiverTurnOrigin,
) -> SessionUpdate {
    SessionUpdate::Xai(Box::new(
        crate::extensions::notification::SessionNotification {
            session_id: info.id.clone(),
            meta: Some(serde_json::json!({
                "_grokParentTask": admission.parent_task, "_receiverTurnOrigin": origin,
            })),
            update: crate::extensions::notification::SessionUpdate::TurnCompleted {
                prompt_id: prompt.into(),
                stop_reason: "end_turn".into(),
                agent_result: None,
                error_kind: None,
                usage: None,
                elapsed_ms: None,
            },
        },
    ))
}

#[tokio::test]
async fn retry_uses_the_actual_committed_occurrence_not_the_retry_prompt() {
    let (_directory, storage, info) = fixture().await;
    let event = event(
        "child",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Completed,
    );
    storage
        .prepare_subagent_completion(&info, event.clone())
        .await
        .unwrap();
    storage
        .publish_subagent_completion(&info, &event, true)
        .await
        .unwrap();
    let owner = task("same-parent-instance");
    let original = projected(&event, "actual-prompt", &owner);
    assert!(matches!(
        storage.append_completion_sync(&info, &original, &owner, |_| Err(io::Error::other(
            "lost durable acknowledgement"
        ))),
        Err(StrictAppendError::Committed { .. })
    ));
    let retry = projected(&event, "not-the-actual-prompt", &owner);
    assert!(matches!(
        storage
            .append_completion_commit_aware(&info, &retry, &owner)
            .await
            .unwrap(),
        CompletionProjectionAck::AlreadyPresent { .. }
    ));
    storage
        .append_update_durable_commit_aware(
            &info,
            &terminal(
                &info,
                "not-the-actual-prompt",
                &owner,
                ReceiverTurnOrigin::Other,
            ),
        )
        .await
        .unwrap();
    let summary = storage.recover_subagent_completions(&info).await.unwrap();
    assert_eq!(
        summary.completion_delivery.receipts[0].prompt_id,
        "actual-prompt"
    );
    assert_eq!(
        summary.completion_delivery.obligations[0]
            .receipt()
            .unwrap()
            .prompt_id,
        "actual-prompt"
    );
    storage
        .append_update_durable_commit_aware(
            &info,
            &terminal(&info, "actual-prompt", &owner, ReceiverTurnOrigin::Other),
        )
        .await
        .unwrap();
    assert!(
        storage
            .recover_subagent_completions(&info)
            .await
            .unwrap()
            .completion_delivery
            .obligations
            .is_empty()
    );
    assert_eq!(
        storage
            .read_chat_history_sync(storage.chat_file(&info), CHAT_FORMAT_VERSION)
            .unwrap()
            .iter()
            .filter(|item| item.text_content() == event.text)
            .count(),
        1
    );
}

#[tokio::test]
async fn restart_continues_existing_projection_and_fences_late_parent_terminals() {
    let (directory, storage, info) = fixture().await;
    let event = event(
        "child",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Completed,
    );
    storage
        .prepare_subagent_completion(&info, event.clone())
        .await
        .unwrap();
    storage
        .publish_subagent_completion(&info, &event, true)
        .await
        .unwrap();
    let old = task("old-parent-instance");
    storage
        .append_completion_commit_aware(&info, &projected(&event, "reused-prompt", &old), &old)
        .await
        .unwrap();
    let restored = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    restored.recover_subagent_completions(&info).await.unwrap();
    let new = task("new-parent-instance");
    let retry = projected(&event, "reused-prompt", &new);
    let CompletionProjectionAck::Continue { history, sources } = restored
        .append_completion_commit_aware(&info, &retry, &new)
        .await
        .unwrap()
    else {
        panic!("unfinished projection must continue")
    };
    assert_eq!(sources.retained.len(), 1);
    assert_eq!(sources.retained[0].source, xai_grok_sampling_types::SourceOccurrence::Unadmitted, "continuation cannot create evidence absent from the original receipt");
    assert_eq!(
        history
            .iter()
            .filter(|item| item.text_content() == event.text)
            .count(),
        1
    );
    restored
        .append_update_durable_commit_aware(
            &info,
            &terminal(&info, "reused-prompt", &old, ReceiverTurnOrigin::Other),
        )
        .await
        .unwrap();
    assert_eq!(
        restored
            .recover_subagent_completions(&info)
            .await
            .unwrap()
            .completion_delivery
            .obligations
            .len(),
        1
    );
    assert!(matches!(
        restored
            .append_completion_commit_aware(&info, &retry, &new)
            .await
            .unwrap(),
        CompletionProjectionAck::AlreadyPresent { .. }
    ));
    restored
        .append_update_durable_commit_aware(
            &info,
            &terminal(&info, "reused-prompt", &new, ReceiverTurnOrigin::Other),
        )
        .await
        .unwrap();
    assert!(
        restored
            .recover_subagent_completions(&info)
            .await
            .unwrap()
            .completion_delivery
            .obligations
            .is_empty()
    );
    let late = task("late-parent-instance");
    assert!(matches!(
        restored
            .append_completion_commit_aware(&info, &projected(&event, "late", &late), &late)
            .await
            .unwrap(),
        CompletionProjectionAck::AlreadyPresent { .. }
    ));
    assert_eq!(
        restored
            .read_chat_history_sync(restored.chat_file(&info), CHAT_FORMAT_VERSION)
            .unwrap()
            .iter()
            .filter(|item| item.text_content() == event.text)
            .count(),
        1
    );
}

#[tokio::test]
async fn closed_parent_admission_cannot_project_or_rebind_a_completion() {
    let (_directory, storage, info) = fixture().await;
    let event = event(
        "child",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Completed,
    );
    storage
        .prepare_subagent_completion(&info, event.clone())
        .await
        .unwrap();
    let closed = task("closed-parent");
    closed.cancellation.cancel();
    assert!(matches!(
        storage
            .append_completion_commit_aware(&info, &projected(&event, "closed", &closed), &closed)
            .await
            .unwrap(),
        CompletionProjectionAck::TurnClosed
    ));
    assert!(
        storage
            .read_chat_history_sync(storage.chat_file(&info), CHAT_FORMAT_VERSION)
            .unwrap()
            .is_empty()
    );
    assert!(
        storage
            .load_summary(&info)
            .await
            .unwrap()
            .completion_delivery
            .receipts
            .is_empty()
    );
}

#[tokio::test]
async fn receiver_recovery_preserves_parent_fencing_and_never_requeues_visible_input() {
    let (directory, storage, info) = fixture().await;
    let (lease, _) = storage.claim_receiver(&info).await.unwrap();
    let request = DeliveryRequest {
        session_id: info.id.to_string(),
        delivery_id: "stable-receiver-id".into(),
        payload: "exact receiver payload".into(),
        action: DeliveryAction::Admit,
    };
    assert!(matches!(
        storage
            .receive_delivery(&info, &request, ReceiverAccess::Owner(&lease))
            .await
            .unwrap(),
        DeliveryResult::Admitted { .. }
    ));
    let owner = task("receiver-parent");
    let projection = DeliveryProjection {
        item: ConversationItem::user(request.payload.clone()),
        binding: DeliveryBinding {
            delivery_id: request.delivery_id.clone(),
            payload_digest: request.digest(),
            turn_id: "receiver-turn".into(),
            parent_task: owner.parent_task.clone(),
        },
    };
    storage
        .append_delivery_commit_aware(&info, &projection, &owner)
        .await
        .unwrap();
    drop(lease);
    let restored = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    let (lease, _) = restored.claim_receiver(&info).await.unwrap();
    let mut observe = request.clone();
    observe.action = DeliveryAction::Observe;
    let DeliveryResult::Admitted { receipt } = restored
        .receive_delivery(&info, &observe, ReceiverAccess::Observe)
        .await
        .unwrap()
    else {
        panic!("durable receiver identity")
    };
    assert!(matches!(
        receipt.progress,
        DeliveryProgress::Interrupted { .. }
    ));
    assert_eq!(receipt.revision, 3);
    assert!(
        restored
            .finish_delivery_turn(
                &info,
                "receiver-turn",
                &task("foreign-parent").parent_task,
                &ReceiverTurnOrigin::Other,
                &DeliveryProgress::Succeeded
            )
            .await
            .unwrap()
            .is_empty()
    );
    let terminal = restored
        .finish_delivery_turn(
            &info,
            "receiver-turn",
            &owner.parent_task,
            &ReceiverTurnOrigin::Other,
            &DeliveryProgress::Succeeded,
        )
        .await
        .unwrap();
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].progress, DeliveryProgress::Succeeded);
    assert_eq!(terminal[0].revision, 4);
    let DeliveryResult::Admitted { receipt } = restored
        .receive_delivery(&info, &request, ReceiverAccess::Owner(&lease))
        .await
        .unwrap()
    else {
        panic!("same admitted identity")
    };
    assert_eq!(receipt.progress, DeliveryProgress::Succeeded);
    assert_eq!(
        restored
            .read_chat_history_sync(restored.chat_file(&info), CHAT_FORMAT_VERSION)
            .unwrap()
            .iter()
            .filter(|item| item.text_content() == request.payload)
            .count(),
        1
    );
}
