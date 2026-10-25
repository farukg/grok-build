use super::*;
use crate::agent::subagent::{SubagentMeta, SubagentMetaStatus};
use crate::extensions::subagent_context::SubagentContext;

#[tokio::test]
async fn producer_commit_materializes_terminal_metadata_without_parent_publication() {
    for (state, status, error, policy) in [
        (
            SubagentState::Completed,
            SubagentMetaStatus::Completed,
            None,
            CompletionEventPolicy::Automatic,
        ),
        (
            SubagentState::Failed {
                message: "upload denied".into(),
            },
            SubagentMetaStatus::Failed,
            Some("upload denied"),
            CompletionEventPolicy::Automatic,
        ),
        (
            SubagentState::Interrupted {
                cause: InterruptionCause::Error {
                    message: "receipt actor failed".into(),
                },
            },
            SubagentMetaStatus::Cancelled,
            Some("receipt actor failed"),
            CompletionEventPolicy::Automatic,
        ),
        (
            SubagentState::Interrupted {
                cause: InterruptionCause::ExplicitStop {
                    actor: xai_tool_types::SubagentActor::Human,
                },
            },
            SubagentMetaStatus::Cancelled,
            Some("Subagent was stopped by human user."),
            CompletionEventPolicy::Silent,
        ),
    ] {
        let (directory, storage, info) = fixture().await;
        let mut outcome = event(
            "child",
            xai_message_delivery_core::AttemptId::mint(1),
            state,
        );
        outcome.policy = policy;
        let metadata = SubagentMeta {
            subagent_id: outcome.id.subagent_id.clone(),
            attempt_id: Some(outcome.id.attempt_id.to_string()),
            parent_session_id: info.id.to_string(),
            child_session_id: outcome.child_session_id.clone(),
            subagent_type: "general-purpose".into(),
            description: "producer metadata".into(),
            prompt: "original child work".into(),
            status: SubagentMetaStatus::Running,
            started_at: outcome.completed_at - chrono::Duration::milliseconds(100),
            completed_at: None,
            duration_ms: None,
            tool_calls: None,
            turns: None,
            error: None,
            context: SubagentContext::Unreported,
            persona: None,
            resumed_from: None,
            child_cwd: Some("/tmp/child".into()),
            worktree_path: Some("/tmp/original-worktree".into()),
            snapshot_ref: Some("refs/grok/child-snapshot".into()),
            effective_model_id: Some("original-model".into()),
        };
        let path = directory.path().join("subagents/child/meta.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        let read =
            || serde_json::from_slice::<SubagentMeta>(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(read().status, SubagentMetaStatus::Running);
        outcome.artifacts = CompletionArtifacts::Metadata {
            metadata: Box::new(metadata.clone()),
        };
        storage
            .prepare_subagent_completion(&info, outcome.clone())
            .await
            .unwrap();
        let terminal = read();
        assert_eq!(terminal.status, status);
        assert_eq!(terminal.error.as_deref(), error);
        assert_eq!(terminal.completed_at, Some(outcome.completed_at));
        assert_eq!(terminal.duration_ms, Some(100));
        assert_eq!(terminal.tool_calls, Some(2));
        assert_eq!(terminal.turns, Some(1));
        assert_eq!(terminal.started_at, metadata.started_at);
        assert_eq!(terminal.snapshot_ref, metadata.snapshot_ref);
        assert_eq!(terminal.worktree_path, metadata.worktree_path);
        assert_eq!(terminal.attempt_id, metadata.attempt_id);
        assert_eq!(terminal.child_session_id, metadata.child_session_id);
        assert_eq!(terminal.effective_model_id, metadata.effective_model_id);
        let summary = storage.load_summary(&info).await.unwrap();
        assert!(matches!(summary.completion_delivery.obligations.as_slice(),
            [CompletionObligation::Preparing { event, .. }] if event.id == outcome.id && event.state == outcome.state && event.completed_at == outcome.completed_at));
        assert!(storage.read_updates_jsonl(storage.updates_file(&info)).unwrap().iter().all(|update|
            !matches!(update, crate::session::storage::SessionUpdate::Xai(notification)
                if matches!(notification.update, crate::extensions::notification::SessionUpdate::SubagentFinished { .. }))));
        let committed = std::fs::read(&path).unwrap();
        outcome.completed_at += chrono::Duration::hours(1);
        let replay = storage
            .prepare_subagent_completion(&info, outcome)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            committed,
            "producer replay preserves the committed timestamp and metadata"
        );
        std::fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        let reloaded =
            JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
        let recovered = reloaded.recover_subagent_completions(&info).await.unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            committed,
            "recovery materializes the committed outcome after a crash before the metadata write"
        );
        match policy {
            CompletionEventPolicy::Automatic | CompletionEventPolicy::BoundaryOnly => {
                assert!(
                    matches!(recovered.completion_delivery.obligations.as_slice(),
                    [CompletionObligation::Pending { id, .. }] if id == &replay.event.id)
                );
            }
            CompletionEventPolicy::Silent => {
                assert!(recovered.completion_delivery.obligations.is_empty());
            }
        }
        let notifications = reloaded
            .read_updates_jsonl(reloaded.updates_file(&info))
            .unwrap();
        let finishes: Vec<_> = notifications.iter().filter(|update| matches!(update,
            crate::session::storage::SessionUpdate::Xai(notification)
                if matches!(notification.update, crate::extensions::notification::SessionUpdate::SubagentFinished { .. }))).collect();
        assert_eq!(finishes.len(), 1);
        let crate::session::storage::SessionUpdate::Xai(finish) = finishes[0] else {
            panic!("the durable finish must carry its exact completion fact");
        };
        let fact: crate::session::completion_delivery::SubagentCompletionFact =
            serde_json::from_value(finish.meta.as_ref().unwrap()["subagentCompletion"].clone())
                .unwrap();
        assert_eq!(fact.id, replay.event.id);
        assert_eq!(fact.state, replay.event.state);
        let updates = std::fs::read(reloaded.updates_file(&info)).unwrap();
        reloaded.recover_subagent_completions(&info).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), committed);
        assert_eq!(
            std::fs::read(reloaded.updates_file(&info)).unwrap(),
            updates
        );
    }
}
