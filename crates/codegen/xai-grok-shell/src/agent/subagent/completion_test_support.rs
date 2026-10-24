use super::completion_publication::{prepare_completion_admission, publish_committed_completion};
use super::*;
#[cfg(feature = "test-support")]
use crate::session::storage::StorageAdapter;
use tokio_util::sync::CancellationToken;
use xai_grok_tools::implementations::grok_build::task::coordinator::ChildCompletion;

#[path = "tests/completion_fixture.rs"]
mod completion_fixture;
#[path = "tests/coordinator_fixture.rs"]
mod coordinator_fixture;
pub(crate) use completion_fixture::{DurableCompletionFixture, completion_data_with_initial_meta};
pub(crate) use coordinator_fixture::CoordinatorFixture;

pub(crate) fn auto_wake_test_request(id: &str) -> SubagentRequest {
    SubagentRequest {
        id: id.into(),
        prompt: String::new(),
        description: "explore".into(),
        subagent_type: "general-purpose".into(),
        parent_session_id: "parent".into(),
        parent_prompt_id: None,
        resume_from: None,
        cwd: None,
        runtime_overrides: Default::default(),
        run_in_background: true,
        surface_completion: true,
        await_to_completion: false,
        fork_context: false,
        owner: SubagentOwner::Task,
        cancel_token: CancellationToken::new(),
        spawn_root: Default::default(),
        tool_call_id: None,
    }
}

pub(crate) fn pending_shell_completion_fixture(
    ctx: &SubagentSpawnContext,
    id: xai_chat_state::SubagentCompletionId,
    meta_dir: &Path,
) -> ShellCompletionData {
    let request = auto_wake_test_request(&id.subagent_id);
    let metadata: SubagentMeta = serde_json::from_value(serde_json::json!({
        "subagent_id": id.subagent_id,
        "attempt_id": id.attempt_id.to_string(),
        "parent_session_id": ctx.parent_session_id,
        "child_session_id": id.subagent_id,
        "subagent_type": request.subagent_type,
        "description": request.description,
        "prompt": request.prompt,
        "status": "running",
        "started_at": "2026-01-01T00:00:00Z",
    }))
    .expect("running child metadata fixture");
    completion_data_with_initial_meta(ctx, metadata, meta_dir)
}

#[cfg(feature = "test-support")]
pub struct CompletionReplay {
    pub parent_session_id: acp::SessionId,
    pub child_session_id: String,
    pub outcome: SubagentResult,
    pub metadata: SubagentMeta,
    pub messages: Vec<acp::ExtNotification>,
}

#[cfg(feature = "test-support")]
pub async fn completion_replay_for_e2e(state: SubagentState) -> CompletionReplay {
    use xai_grok_tools::implementations::grok_build::task::backend::SubagentBackend;
    use xai_grok_tools::implementations::grok_build::task::coordinator::TerminalVisibilityResult;
    let mut ctx = super::isolated_spawn_e2e::spawn_ctx(PathBuf::from("/tmp"));
    ctx.parent_session_id = "completion-replay-parent".into();
    let fixture = DurableCompletionFixture::new(&mut ctx).await;
    let mut request = auto_wake_test_request("completion-replay-child");
    request.parent_session_id = ctx.parent_session_id.clone();
    let parent_session_id = fixture.info.id.clone();
    let child_session_id = request.id.clone();
    let meta_dir = fixture.directory.path().join("subagents").join(&request.id);
    let (gateway_tx, mut gateway_rx) = mpsc::unbounded_channel();
    let gateway = GatewaySender::new(gateway_tx);
    let mut coordinator = CoordinatorFixture::new(ctx, state, meta_dir.clone(), gateway.clone());
    let backend = coordinator.backend.clone();
    let spawned = tokio::task::spawn_local(async move { backend.spawn(request, None, None).await });
    let completion = coordinator
        .classified
        .recv()
        .await
        .expect("real coordinator classification");
    super::spawn::emit_subagent_notification(
        &gateway,
        &parent_session_id.to_string(),
        SessionUpdate::SubagentSpawned {
            subagent_id: child_session_id.clone(),
            attempt_id: Some(completion.attempt_id.to_string()),
            parent_session_id: parent_session_id.to_string(),
            parent_prompt_id: None,
            child_session_id: child_session_id.clone(),
            subagent_type: completion.request.subagent_type.clone(),
            description: completion.request.description.clone(),
            context: SubagentContext::Fresh,
            capability_mode: None,
            persona: None,
            role: None,
            model: None,
            resumed_from: None,
            workflow_run_id: None,
            agent_address: None,
            resume_fallback: None,
        },
        None,
    );
    let published = coordinator
        .visible
        .recv()
        .await
        .expect("real publication result");
    assert!(matches!(published, TerminalVisibilityResult::Published));
    let outcome = spawned
        .await
        .expect("coordinator run")
        .expect("native terminal outcome");
    assert!(matches!(
        publish_committed_completion(completion, gateway.clone()).await,
        TerminalVisibilityResult::Published
    ));
    let marker = acp::ExtNotification::new(
        "fixture/completion-replay-fifo",
        serde_json::value::to_raw_value(&serde_json::Value::Null)
            .unwrap()
            .into(),
    );
    gateway.forward_fire_and_forget(marker);
    let mut messages = Vec::new();
    loop {
        let message = gateway_rx.recv().await.expect("gateway FIFO marker");
        if let xai_acp_lib::AcpClientMessage::ExtNotification(args) = message {
            if args.request.method.as_ref() == "fixture/completion-replay-fifo" {
                break;
            }
            messages.push(args.request);
        }
    }
    let metadata =
        serde_json::from_slice(&std::fs::read(meta_dir.join("meta.json")).unwrap()).unwrap();
    let persisted = fixture
        .storage
        .load_session(&fixture.info)
        .await
        .expect("actual persisted replay");
    assert_eq!(persisted.updates.iter().filter(|update| matches!(update,
        crate::session::storage::SessionUpdate::Xai(notification) if matches!(&notification.update,
            SessionUpdate::SubagentFinished { subagent_id, .. } if subagent_id == &child_session_id))).count(), 1);
    CompletionReplay {
        parent_session_id,
        child_session_id,
        outcome,
        metadata,
        messages,
    }
}
