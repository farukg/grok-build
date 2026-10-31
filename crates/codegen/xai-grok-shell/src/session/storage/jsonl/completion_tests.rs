use super::*;

#[path = "completion_batch_tests.rs"]
mod batch;
#[path = "completion_metadata_tests.rs"]
mod metadata;
#[path = "completion_recovery_tests.rs"]
mod recovery;
#[path = "terminal_restart_tests.rs"]
mod terminal_restart;
#[path = "restart_cohort_tests.rs"]
mod restart_cohort;
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
async fn cold_publications_seed_from_parent_history_before_replay_and_retry_exactly() {
    let (_directory, storage, info) = fixture().await;
    let frontier = crate::util::event_id::next_event_sequence()
        .checked_add(1_000_000)
        .unwrap();
    let history = serde_json::json!({
        "timestamp": 1,
        "method": "session/update",
        "params": {
            "sessionId": info.id.to_string(),
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "retained history" }
            },
            "_meta": { "eventId": format!("{}-{frontier}", info.id) }
        }
    });
    let mut original = serde_json::to_vec(&history).unwrap();
    original.push(b'\n');
    std::fs::write(storage.updates_file(&info), &original).unwrap();
    let first = event(
        "cold-first",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Completed,
    );
    let second = event(
        "cold-second",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Completed,
    );
    for event in [&first, &second] {
        storage.prepare_subagent_completion(&info, event.clone()).await.unwrap();
    }
    let (first_publication, second_publication) = tokio::join!(
        storage.publish_subagent_completion(&info, &first, true),
        storage.publish_subagent_completion(&info, &second, true),
    );
    let first_publication = first_publication.unwrap();
    let second_publication = second_publication.unwrap();
    let sequence = |notification: &crate::extensions::notification::SessionNotification| {
        notification.meta.as_ref().unwrap()["eventId"]
            .as_str().unwrap().rsplit('-').next().unwrap().parse::<u64>().unwrap()
    };
    assert!(sequence(&first_publication.notification) > frontier);
    assert!(sequence(&second_publication.notification) > frontier);
    assert_ne!(sequence(&first_publication.notification), sequence(&second_publication.notification));
    let committed = std::fs::read(storage.updates_file(&info)).unwrap();
    assert!(committed.starts_with(&original));
    let retry = storage.publish_subagent_completion(&info, &first, false).await.unwrap();
    assert_eq!(
        serde_json::to_value(&retry.notification).unwrap(),
        serde_json::to_value(&first_publication.notification).unwrap(),
    );
    assert_eq!(std::fs::read(storage.updates_file(&info)).unwrap(), committed);
    assert_eq!(storage.read_updates_jsonl(storage.updates_file(&info)).unwrap().len(), 3);
}

#[tokio::test]
async fn retry_preserves_historical_low_completion_identity() {
    let (_directory, storage, info) = fixture().await;
    let completed = event(
        "historical-child",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Completed,
    );
    storage.prepare_subagent_completion(&info, completed.clone()).await.unwrap();
    let mut old = crate::agent::subagent::completion_lifecycle_notification_from_event(
        &completed,
        &info.id.to_string(),
        true,
        0,
    )
    .unwrap();
    old.meta.as_mut().unwrap()["eventId"] = serde_json::json!(format!("{}-49", info.id));
    let update = crate::session::storage::SessionUpdate::Xai(Box::new(old.clone()));
    let mut retained = serde_json::to_vec(&SessionUpdateEnvelope::from_update(&update).unwrap()).unwrap();
    retained.push(b'\n');
    std::fs::write(storage.updates_file(&info), &retained).unwrap();
    let retry = storage.publish_subagent_completion(&info, &completed, false).await.unwrap();
    assert_eq!(serde_json::to_value(retry.notification).unwrap(), serde_json::to_value(old).unwrap());
    assert_eq!(std::fs::read(storage.updates_file(&info)).unwrap(), retained);
}

#[tokio::test]
async fn exhausted_journal_rejects_new_publication_but_preserves_low_id_retry() {
    let (_directory, storage, info) = fixture().await;
    let completed = event(
        "existing-child",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Completed,
    );
    let pending = event(
        "new-child",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Completed,
    );
    for event in [&completed, &pending] {
        storage
            .prepare_subagent_completion(&info, event.clone())
            .await
            .unwrap();
    }
    let mut old = crate::agent::subagent::completion_lifecycle_notification_from_event(
        &completed,
        &info.id.to_string(),
        true,
        0,
    )
    .unwrap();
    old.meta.as_mut().unwrap()["eventId"] = serde_json::json!(format!("{}-49", info.id));
    let update = crate::session::storage::SessionUpdate::Xai(Box::new(old.clone()));
    let mut retained = serde_json::to_vec(
        &SessionUpdateEnvelope::from_update(&update).unwrap(),
    )
    .unwrap();
    retained.push(b'\n');
    let maximum = serde_json::json!({
        "timestamp": 1,
        "method": "session/update",
        "params": {
            "sessionId": info.id.to_string(),
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "retained maximum" }
            },
            "_meta": { "eventId": format!("{}-18446744073709551615", info.id) }
        }
    });
    retained.extend(serde_json::to_vec(&maximum).unwrap());
    retained.push(b'\n');
    std::fs::write(storage.updates_file(&info), &retained).unwrap();

    assert!(storage
        .publish_subagent_completion(&info, &pending, true)
        .await
        .is_err());
    assert_eq!(std::fs::read(storage.updates_file(&info)).unwrap(), retained);

    let retry = storage
        .publish_subagent_completion(&info, &completed, false)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(retry.notification).unwrap(),
        serde_json::to_value(old).unwrap(),
    );
    assert_eq!(std::fs::read(storage.updates_file(&info)).unwrap(), retained);
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
    let CompletionProjectionAck::AlreadyPresent { history, sources } = ack else {
        panic!("same completion must reconcile existing projection")
    };
    assert_eq!(sources.retained.len(), 1);
    assert_eq!(sources.retained[0].source, xai_grok_sampling_types::SourceOccurrence::Unadmitted, "a legacy receipt has no admitted source, even when its body is retained");
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
