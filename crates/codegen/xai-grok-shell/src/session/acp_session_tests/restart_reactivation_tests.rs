use super::support::*;
use super::*;
use crate::session::receiver_execution::ReceiverAccess;
use crate::session::storage::{StorageAdapter, jsonl::JsonlStorageAdapter};
use std::sync::atomic::{AtomicBool, Ordering};
use xai_grok_tools::types::restart_recovery::{LeaderRestartRecovery, RestartAdmission};

struct Fixture {
    actor: Arc<SessionActor>,
    storage: JsonlStorageAdapter,
    fail_activation: Arc<AtomicBool>,
    _directory: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let (gateway, _received) = mpsc::unbounded_channel();
    let (persistence, mut commands) = mpsc::unbounded_channel();
    let mut actor = create_test_actor(0, 256_000, 85, gateway, persistence).await;
    actor.hook_resolved_workspace_root = "/tmp".into();
    let directory = tempfile::tempdir().unwrap();
    let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    let info = actor.session_info.clone();
    storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
    let (lease, _) = storage.claim_receiver(&info).await.unwrap();
    let startup = LeaderRestartRecovery::stale(chrono::Utc::now());
    actor.tool_context.restart_recovery = startup.register(
        info.id.0.as_ref(), chrono::Utc::now() - chrono::Duration::hours(2), Default::default(),
    );
    storage.set_restart_admission(&info, actor.tool_context.restart_recovery.admission(), ReceiverAccess::Owner(&lease)).await.unwrap();
    let actor = Arc::new(actor);
    *actor.hook_registry.borrow_mut() = Some(Arc::new(super::prompt_gate_tests::prompt_gate_registry("exit 2")));
    let fail_activation = Arc::new(AtomicBool::new(false));
    let fault = fail_activation.clone();
    let durable = storage.clone();
    tokio::task::spawn_local(async move {
        while let Some(command) = commands.recv().await {
            match command {
                PersistenceMsg::GetCompletionState { respond_to } => {
                    let _ = respond_to.send(durable.read_completion_state(&info).await);
                }
                PersistenceMsg::SetRestartAdmission { admission, respond_to } => {
                    let result = if matches!(admission, RestartAdmission::Reactivated { .. }) && fault.swap(false, Ordering::SeqCst) {
                        Err(std::io::Error::other("injected admission persistence failure"))
                    } else {
                        durable.set_restart_admission(&info, admission, ReceiverAccess::Owner(&lease)).await
                    };
                    let _ = respond_to.send(result);
                }
                PersistenceMsg::SuppressTerminalCompletions { ids, reason, respond_to } => {
                    let _ = respond_to.send(durable.suppress_terminal_completions(&info, ids, reason).await);
                }
                PersistenceMsg::FlushAndAck { respond_to } => {
                    let _ = respond_to.send(Ok(()));
                }
                _ => {},
            }
        }
    });
    Fixture { actor, storage, fail_activation, _directory: directory }
}

async fn prompt(actor: &Arc<SessionActor>, id: &str, text: &str) -> PromptTurnResult {
    Box::pin(actor.handle_prompt(
        id, vec![acp::ContentBlock::Text(acp::TextContent::new(text))], PromptMode::Agent,
        None, None, None, None, false, false, None, None, None,
    )).await
}

#[tokio::test(flavor = "current_thread")]
async fn only_a_new_message_unlocks_the_actual_turn_entry_point() {
    tokio::task::LocalSet::new().run_until(async {
        let fixture = fixture().await;
        assert!(prompt(&fixture.actor, "scheduler-fired-old", "recover old work").await.is_err());
        fixture.actor.resume_durable_deliveries(crate::session::delivery_receiver::ReceiverResume::Recovery { deliveries: Vec::new() }).await;
        assert!(!fixture.actor.restart_recovery_permitted());
        prompt(&fixture.actor, "user-command", "/goal resume").await.unwrap();
        assert!(!fixture.actor.restart_recovery_permitted());
        let result = prompt(&fixture.actor, "new-user-message", "This is newly submitted user work.").await.unwrap();
        assert_eq!(result.stop_reason, acp::StopReason::Cancelled);
        assert!(fixture.actor.restart_recovery_permitted());
        let durable = fixture.storage.read_completion_state(&fixture.actor.session_info).await.unwrap();
        assert!(matches!(durable.restart_admission, RestartAdmission::Reactivated { .. }));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn failed_activation_persistence_stays_quiescent_until_user_retries() {
    tokio::task::LocalSet::new().run_until(async {
        let fixture = fixture().await;
        fixture.fail_activation.store(true, Ordering::SeqCst);
        assert!(prompt(&fixture.actor, "first-user-message", "new work").await.is_err());
        assert!(!fixture.actor.restart_recovery_permitted());
        let durable = fixture.storage.read_completion_state(&fixture.actor.session_info).await.unwrap();
        assert!(matches!(durable.restart_admission, RestartAdmission::Quiescent { .. }));
        assert!(prompt(&fixture.actor, "plan-resume-old", "not a user retry").await.is_err());
        assert!(!fixture.actor.restart_recovery_permitted());
        let result = prompt(&fixture.actor, "second-user-message", "retry the new work").await.unwrap();
        assert_eq!(result.stop_reason, acp::StopReason::Cancelled);
        assert!(fixture.actor.restart_recovery_permitted());
    }).await;
}
