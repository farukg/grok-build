use super::*;

#[path = "completion_batch_tests.rs"]
mod batch;
#[path = "completion_recovery_tests.rs"]
mod recovery;
use crate::session::completion_delivery::{
    CompletionArtifacts, CompletionEventPolicy, CompletionObligation, CompletionWakePolicy,
};
use crate::session::storage::StorageAdapter;
use xai_chat_state::{CompletionProjection, CompletionProjectionAck, SubagentCompletionId};
use xai_grok_tools::implementations::grok_build::task::types::{InterruptionCause, SubagentState};

fn admission() -> xai_chat_state::InputAdmission {
    xai_chat_state::InputAdmission {
        parent_task: xai_chat_state::ParentTask::Unknown,
        cancellation: tokio_util::sync::CancellationToken::new(),
    }
}

async fn fixture() -> (tempfile::TempDir, JsonlStorageAdapter, Info) {
    let directory = tempfile::tempdir().unwrap();
    let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    let info = Info {
        id: acp::SessionId::new("durable-completion-parent"),
        cwd: "/tmp".into(),
    };
    storage
        .init_session(&info, acp::ModelId::new("test"))
        .await
        .unwrap();
    (directory, storage, info)
}
fn event(
    id: &str,
    attempt: xai_message_delivery_core::AttemptId,
    state: SubagentState,
) -> SubagentCompletionEvent {
    SubagentCompletionEvent {
        id: SubagentCompletionId {
            subagent_id: id.into(),
            attempt_id: attempt,
        },
        text: format!("actual fixture result {id}: {}", state.model_text()),
        state,
        output: "durable child output".into(),
        child_session_id: id.into(),
        tool_calls: 2,
        turns: 1,
        duration_ms: 100,
        tokens_used: 11,
        completed_at: chrono::Utc::now(),
        policy: CompletionEventPolicy::Automatic,
        artifacts: CompletionArtifacts::None,
    }
}

#[tokio::test]
async fn committed_projection_error_retries_without_a_second_model_input() {
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
    let projection =
        CompletionProjection::new(event.id.clone(), "parent-turn".into(), event.text.clone());
    let error = storage
        .append_completion_sync(&info, &projection, &admission(), |_| {
            Err(io::Error::other("injected file sync failure"))
        })
        .unwrap_err();
    assert!(matches!(
        error,
        StrictAppendError::Committed {
            acknowledgement: CompletionProjectionAck::Appended,
            ..
        }
    ));
    let ack = storage
        .append_completion_sync(&info, &projection, &admission(), |file| file.sync_all())
        .unwrap();
    let CompletionProjectionAck::AlreadyPresent { history } = ack else {
        panic!("same completion must reconcile existing projection")
    };
    assert_eq!(
        history
            .iter()
            .filter(|item| item.text_content() == event.text)
            .count(),
        1
    );
    let published = storage
        .publish_subagent_completion(&info, &event, true)
        .await
        .unwrap();
    assert_eq!(
        published.offer,
        crate::session::completion_delivery::CompletionOffer::None
    );
    let summary = storage.recover_subagent_completions(&info).await.unwrap();
    assert_eq!(summary.completion_delivery.receipts.len(), 1);
}

#[tokio::test]
async fn child_id_reuse_has_distinct_attempt_receipts_after_history_rewrite() {
    let (_directory, storage, info) = fixture().await;
    let first = event(
        "same-child",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Completed,
    );
    let second = event(
        "same-child",
        xai_message_delivery_core::AttemptId::mint(2),
        SubagentState::Interrupted {
            cause: InterruptionCause::Error {
                message: "real second-attempt error".into(),
            },
        },
    );
    for event in [&first, &second] {
        storage
            .prepare_subagent_completion(&info, event.clone())
            .await
            .unwrap();
        storage
            .publish_subagent_completion(&info, event, true)
            .await
            .unwrap();
        assert!(matches!(
            storage
                .append_completion_commit_aware(
                    &info,
                    &CompletionProjection::new(event.id.clone(), "turn".into(), event.text.clone()),
                    &admission()
                )
                .await
                .unwrap(),
            CompletionProjectionAck::Appended
        ));
    }
    storage
        .replace_chat_history(
            &info,
            &[ConversationItem::assistant("compacted retained context")],
        )
        .await
        .unwrap();
    for event in [&first, &second] {
        assert!(matches!(
            storage
                .append_completion_commit_aware(
                    &info,
                    &CompletionProjection::new(
                        event.id.clone(),
                        "retry".into(),
                        event.text.clone()
                    ),
                    &admission()
                )
                .await
                .unwrap(),
            CompletionProjectionAck::AlreadyPresent { .. }
        ));
    }
    let summary = storage.load_summary(&info).await.unwrap();
    assert_eq!(summary.completion_delivery.receipts.len(), 2);
    assert!(
        summary
            .completion_delivery
            .receipts
            .iter()
            .any(|receipt| receipt.completion == first.id)
    );
    assert!(
        summary
            .completion_delivery
            .receipts
            .iter()
            .any(|receipt| receipt.completion == second.id)
    );
}

#[tokio::test]
async fn preparation_recovers_success_and_runtime_error_once_and_preserves_human_stop() {
    let (_directory, storage, info) = fixture().await;
    storage
        .set_completion_wake_policy(&info, CompletionWakePolicy::HumanStopped)
        .await
        .unwrap();
    let events = [
        event(
            "success",
            xai_message_delivery_core::AttemptId::mint(1),
            SubagentState::Completed,
        ),
        event(
            "error",
            xai_message_delivery_core::AttemptId::mint(1),
            SubagentState::Interrupted {
                cause: InterruptionCause::Error {
                    message: "child transport failed".into(),
                },
            },
        ),
    ];
    for event in &events {
        storage
            .prepare_subagent_completion(&info, event.clone())
            .await
            .unwrap();
    }
    let recovered = storage.recover_subagent_completions(&info).await.unwrap();
    assert_eq!(
        recovered.completion_delivery.wake_policy,
        CompletionWakePolicy::HumanStopped
    );
    assert_eq!(recovered.completion_delivery.obligations.len(), 2);
    for event in &events {
        assert!(recovered.completion_delivery.obligations.iter().any(|obligation| matches!(obligation,
            CompletionObligation::Pending { id, payload } if id == &event.id && payload.text.contains(&event.state.model_text()))));
    }
    let first_log = std::fs::read(storage.updates_file(&info)).unwrap();
    storage.recover_subagent_completions(&info).await.unwrap();
    assert_eq!(
        std::fs::read(storage.updates_file(&info)).unwrap(),
        first_log
    );
}

#[tokio::test]
async fn successful_caller_terminal_settles_projection_but_parent_errors_remain_recoverable() {
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
    storage
        .append_completion_commit_aware(
            &info,
            &CompletionProjection::new(event.id.clone(), "parent-turn".into(), event.text.clone()),
            &admission(),
        )
        .await
        .unwrap();
    let terminal = |stop, result| {
        crate::session::storage::SessionUpdate::Xai(Box::new(
            crate::extensions::notification::SessionNotification {
                session_id: info.id.clone(),
                meta: None,
                update: crate::extensions::notification::SessionUpdate::TurnCompleted {
                    prompt_id: "parent-turn".into(),
                    stop_reason: stop,
                    agent_result: result,
                    error_kind: None,
                    usage: None,
                    elapsed_ms: Some(1),
                },
            },
        ))
    };
    storage
        .append_update_durable_commit_aware(
            &info,
            &terminal("error".into(), Some("parent failed before sampling".into())),
        )
        .await
        .unwrap();
    assert_eq!(
        storage
            .recover_subagent_completions(&info)
            .await
            .unwrap()
            .completion_delivery
            .obligations
            .len(),
        1
    );
    storage
        .append_update_durable_commit_aware(&info, &terminal("end_turn".into(), None))
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
}
