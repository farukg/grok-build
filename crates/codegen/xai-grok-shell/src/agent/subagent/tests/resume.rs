use super::*;
use crate::session::storage::StorageAdapter;
use crate::test_support::lsp_runtime::test_gateway;
use xai_grok_tools::implementations::grok_build::task::backend::ChannelBackend;
use xai_grok_tools::implementations::grok_build::task::resume::{
    MIN_SUBAGENT_ID_PREFIX_LEN, SubagentReferenceMatch, match_subagent_reference,
};

fn unique_parent(tag: &str) -> String {
    format!("resume-{tag}-{}", uuid::Uuid::now_v7())
}

fn parent_cwd() -> PathBuf {
    std::env::temp_dir()
}

fn subagents_dir(parent: &str) -> PathBuf {
    session::persistence::session_dir(&SessionInfo {
        id: acp::SessionId::new(parent),
        cwd: parent_cwd().to_string_lossy().into_owned(),
    })
    .join("subagents")
}

fn child_meta(id: &str, parent: &str, status: SubagentMetaStatus) -> SubagentMeta {
    SubagentMeta {
        subagent_id: id.to_owned(),
        attempt_id: None,
        parent_session_id: parent.to_owned(),
        child_session_id: id.to_owned(),
        subagent_type: "general-purpose".to_owned(),
        description: "task".to_owned(),
        prompt: "work".to_owned(),
        status,
        started_at: chrono::Utc::now(),
        completed_at: None,
        duration_ms: None,
        tool_calls: None,
        turns: None,
        error: None,
        context: SubagentContext::Unreported,
        persona: None,
        resumed_from: None,
        child_cwd: Some(parent_cwd().to_string_lossy().into_owned()),
        worktree_path: None,
        snapshot_ref: None,
        effective_model_id: Some("test-model".to_owned()),
    }
}

async fn write_child_transcript(id: &str) {
    let info = SessionInfo {
        id: acp::SessionId::new(id),
        cwd: parent_cwd().to_string_lossy().into_owned(),
    };
    let storage = crate::session::storage::jsonl::JsonlStorageAdapter::with_root(
        crate::util::grok_home::grok_home(),
    );
    storage
        .init_session(&info, acp::ModelId::new("test-model"))
        .await
        .expect("child session");
    storage
        .append_chat_message(&info, &ConversationItem::system("prior system"))
        .await
        .expect("system item");
    storage
        .append_chat_message(&info, &ConversationItem::assistant("prior answer"))
        .await
        .expect("assistant item");
}

/// After a parent reload the coordinator holds nothing; a finished child is found on disk by a
/// prefix and its transcript is what the continuation starts from.
#[tokio::test]
async fn resume_after_reload_restores_history_from_disk() {
    let parent = unique_parent("reload");
    let id = uuid::Uuid::now_v7().to_string();
    write_subagent_meta(
        &subagents_dir(&parent).join(&id),
        &child_meta(&id, &parent, SubagentMetaStatus::Completed),
    );
    write_child_transcript(&id).await;

    let prefix = &id[..MIN_SUBAGENT_ID_PREFIX_LEN + 6];
    let candidates = durable_resume_candidates(prefix, &parent, &parent_cwd());
    assert_eq!(
        match_subagent_reference(prefix, candidates.iter().map(|c| (c.as_str(), c.as_str()))),
        SubagentReferenceMatch::Unique(id.clone())
    );
    let source = durable_resume_source(&id, &parent, &parent_cwd()).expect("resumable from disk");
    let child_info = SessionInfo {
        id: acp::SessionId::new(id.clone()),
        cwd: source.child_cwd.clone(),
    };
    let mut request = auto_wake_test_request(&id);
    request.resume_from = Some(id.clone());
    let ctx = ctx_with_toggle(HashMap::new());
    let BootstrapInitialContext::Ready(initial) = bootstrap_initial_context(
        &request,
        Some(&source),
        &ctx,
        &child_info,
        &session::persistence::session_dir(&child_info),
        "test-model",
        super::super::resume_window::ResumeWindowPolicy {
            context_window: 256_000,
            auto_compact_threshold_percent: 85,
        },
    )
    .await
    else {
        panic!("resume bootstrap must load the persisted transcript");
    };
    assert_eq!(initial.context, SubagentContext::Resumed);
    assert!(
        initial
            .conversation
            .iter()
            .any(|item| format!("{item:?}").contains("prior answer")),
        "the old history must be in the resumed context"
    );
    let _ = std::fs::remove_dir_all(subagents_dir(&parent).parent().expect("session dir"));
}

/// A child left `running` by a dead process refuses with a typed reason until the reload heal
/// marks it cancelled, and resumes afterwards.
#[tokio::test]
async fn resume_restart_interrupted_child_by_user() {
    let parent = unique_parent("restart");
    let id = uuid::Uuid::now_v7().to_string();
    let dir = subagents_dir(&parent);
    write_subagent_meta(
        &dir.join(&id),
        &child_meta(&id, &parent, SubagentMetaStatus::Running),
    );

    assert_eq!(
        durable_resume_source(&id, &parent, &parent_cwd()).map(|_| ()),
        Err(SubagentResumeError::NotTerminal {
            subagent_id: id.clone()
        })
    );

    let gateway = test_gateway();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let backend = ChannelBackend::new(event_tx);
    let respond = async move {
        let event = event_rx.recv().await.expect("inspection event");
        let SubagentEvent::Inspect(request) = event else {
            panic!("expected Inspect event");
        };
        let _ = request.respond_to.send(None);
    };
    tokio::join!(
        reconcile_orphaned_subagents_with_backend(
            &[],
            &backend,
            dir.parent().expect("session dir"),
            &parent,
            &gateway,
            None,
            ORPHAN_RECONCILE_REASON,
            std::sync::Arc::new(tokio::sync::Mutex::new(())),
        ),
        respond,
    );

    let source = durable_resume_source(&id, &parent, &parent_cwd())
        .expect("a healed restart orphan is resumable");
    assert_eq!(source.subagent_id, id);
    let _ = std::fs::remove_dir_all(dir.parent().expect("session dir"));
}

#[test]
fn resume_non_terminal_meta_reports_not_terminal_and_foreign_parent() {
    let parent = unique_parent("typed");
    let running = uuid::Uuid::now_v7().to_string();
    let foreign = uuid::Uuid::now_v7().to_string();
    let dir = subagents_dir(&parent);
    write_subagent_meta(
        &dir.join(&running),
        &child_meta(&running, &parent, SubagentMetaStatus::Running),
    );
    write_subagent_meta(
        &dir.join(&foreign),
        &child_meta(&foreign, "another-parent", SubagentMetaStatus::Completed),
    );
    assert!(matches!(
        durable_resume_source(&running, &parent, &parent_cwd()),
        Err(SubagentResumeError::NotTerminal { .. })
    ));
    assert!(matches!(
        durable_resume_source(&foreign, &parent, &parent_cwd()),
        Err(SubagentResumeError::ForeignParent { .. })
    ));
    assert!(matches!(
        durable_resume_source("missing", &parent, &parent_cwd()),
        Err(SubagentResumeError::NotFound { .. })
    ));
    let _ = std::fs::remove_dir_all(dir.parent().expect("session dir"));
}

/// Prefix lookup reads only directory names, so it stays fast with many children.
#[test]
fn resume_by_unique_prefix_is_bounded_with_many_children() {
    let parent = unique_parent("perf");
    let dir = subagents_dir(&parent);
    let ids: Vec<String> = (0..1000)
        .map(|_| uuid::Uuid::now_v7().to_string())
        .collect();
    for id in &ids {
        std::fs::create_dir_all(dir.join(id)).expect("child dir");
    }
    let target = ids.last().expect("ids");
    let prefix = target.get(..target.len() - 2).expect("prefix");
    let started = std::time::Instant::now();
    let candidates = durable_resume_candidates(prefix, &parent, &parent_cwd());
    let elapsed = started.elapsed();
    assert!(candidates.contains(target));
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "prefix lookup over 1000 children took {elapsed:?}"
    );
    let _ = std::fs::remove_dir_all(dir.parent().expect("session dir"));
}

#[test]
fn resume_cwd_fallback_is_reported_to_the_model() {
    let source = ResumeSourceData {
        subagent_id: "sub-gone".into(),
        child_session_id: "sub-gone".into(),
        child_cwd: "/no/such/dir/grok-resume-missing".into(),
        worktree_path: None,
        snapshot_ref: None,
        subagent_type: "general-purpose".into(),
        persona: None,
        model_id: None,
    };
    let fallback = resume_cwd_fallback(Some(&source)).expect("missing cwd is reported");
    assert_eq!(
        fallback,
        xai_tool_types::SubagentResumeFallback::SourceCwdMissing {
            source_cwd: source.child_cwd.clone(),
        }
    );
    let completed = xai_tool_types::SubagentCompletedOutput {
        output: "done".into(),
        subagent_id: "sub-new".into(),
        subagent_type: "general-purpose".into(),
        tool_calls: 0,
        turns: 1,
        duration_ms: 1,
        worktree_path: None,
        persona: None,
        resume_from_hint: "sub-new".into(),
        persona_hint: None,
        resume_fallback: Some(fallback.clone()),
    };
    assert!(completed.to_model_text().contains(&fallback.to_string()));
    let present = ResumeSourceData {
        child_cwd: parent_cwd().to_string_lossy().into_owned(),
        ..source
    };
    assert_eq!(resume_cwd_fallback(Some(&present)), None);
}
