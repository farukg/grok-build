use super::*;
use crate::agent::mvp_agent::agent_ops::SessionInstallOutcome;
use crate::session::completion_delivery::CompletionWakePolicy;

async fn spawn_install(
    agent: &MvpAgent,
    sid: &acp::SessionId,
    cwd: &std::path::Path,
    waiter: Option<&tokio::sync::watch::Receiver<bool>>,
    policy: CompletionWakePolicy,
) -> SessionInstallOutcome {
    agent.set_auth_method(acp::AuthMethodId::new("cached_token"));
    let cwd = xai_grok_paths::AbsPathBuf::new(cwd.to_path_buf()).unwrap();
    let info = SessionInfo {
        id: sid.clone(),
        cwd: cwd.as_str().to_owned(),
    };
    let mut options = chat_session_spawn_options(
        info,
        cwd,
        None,
        None,
        agent.models_manager.current_model_id(),
        false,
    );
    options.is_chat_kind = false;
    options.attach_waiter = waiter;
    options.completion_wake_policy = policy;
    if waiter.is_some() && !matches!(agent.session_host(sid), Some(SessionHost::Child(_))) {
        options.root_identity = Some(test_root_identity(101, 202));
    }
    let init = acp::InitializeRequest::new(acp::ProtocolVersion::V1).client_capabilities(
        acp::ClientCapabilities::new()
            .fs(acp::FileSystemCapabilities::new())
            .terminal(false),
    );
    agent
        .spawn_and_register_session(&init, options, None)
        .await
        .expect("actor construction succeeds independently of registry admission")
}

fn mark_child(agent: &MvpAgent, sid: &acp::SessionId) {
    agent.session_registry.set_host(
        sid,
        SessionHost::Child(ChildHost {
            parent_session_id: acp::SessionId::new("install-test-parent"),
            reach: ChildReach::Unaddressed,
        }),
    );
}

fn assert_stopped(handle: &SessionHandle) {
    assert!(handle.tool_context.task_wake_suppressed.as_ref().unwrap().get());
}

#[test]
fn first_install_reports_installed_with_the_registered_actor() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = acp::SessionId::new(uuid::Uuid::now_v7().to_string());
        assert!(matches!(
            spawn_install(&agent, &sid, cwd.path(), None, CompletionWakePolicy::Automatic).await,
            SessionInstallOutcome::Installed
        ));
        assert!(agent.is_resident(&sid));
        assert!(!agent.resident_handle(&sid).unwrap().cmd_tx.is_closed());
        agent.remove_session(&sid);
    });
}

#[test]
fn replacing_an_old_child_handle_is_installed_and_preserves_restored_human_stop() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = acp::SessionId::new(uuid::Uuid::now_v7().to_string());
        assert!(matches!(
            spawn_install(&agent, &sid, cwd.path(), None, CompletionWakePolicy::Automatic).await,
            SessionInstallOutcome::Installed
        ));
        mark_child(&agent, &sid);
        let old = agent.resident_handle(&sid).unwrap();
        let guard = agent.begin_session_load(&sid);
        assert!(matches!(
            spawn_install(
                &agent, &sid, cwd.path(), Some(&guard.rx), CompletionWakePolicy::HumanStopped,
            ).await,
            SessionInstallOutcome::Installed
        ));
        let current = agent.resident_handle(&sid).unwrap();
        assert!(!current.cmd_tx.same_channel(&old.cmd_tx));
        assert_stopped(&current);
        drop(guard);
        let settled = agent.resident_handle(&sid).unwrap();
        assert!(settled.cmd_tx.same_channel(&current.cmd_tx));
        assert_stopped(&settled);
        agent.remove_session(&sid);
    });
}

async fn newer_registry_winner_survives(point: AttachPause) {
    let agent = build_minimal_agent_for_tests();
    let cwd = tempfile::tempdir().unwrap();
    let sid = acp::SessionId::new(uuid::Uuid::now_v7().to_string());
    mark_child(&agent, &sid);
    let first_guard = agent.begin_session_load(&sid);
    let (first, reached, release) = with_pause_at(
        point,
        spawn_install(
            &agent, &sid, cwd.path(), Some(&first_guard.rx), CompletionWakePolicy::Automatic,
        ),
    );
    let replace = async {
        reached.await.unwrap();
        let successor = agent.begin_session_load(&sid);
        assert!(matches!(
            spawn_install(
                &agent, &sid, cwd.path(), Some(&successor.rx), CompletionWakePolicy::HumanStopped,
            ).await,
            SessionInstallOutcome::Installed
        ));
        let winner = agent.resident_handle(&sid).unwrap();
        assert_stopped(&winner);
        drop(successor);
        release.send(()).unwrap();
        winner
    };
    let (result, winner) = tokio::join!(first, replace);
    assert!(matches!(result, SessionInstallOutcome::Superseded));
    drop(first_guard);
    let current = agent.resident_handle(&sid).unwrap();
    assert!(current.cmd_tx.same_channel(&winner.cmd_tx));
    assert!(!current.cmd_tx.is_closed());
    assert_stopped(&current);
    assert!(matches!(agent.session_host(&sid), Some(SessionHost::Child(_))));
    agent.remove_session(&sid);
}

#[test]
fn refused_old_attach_cannot_change_the_new_actor_or_wake_policy() {
    run_local_for_bridge_test(|| newer_registry_winner_survives(AttachPause::AfterSpawn));
}

#[test]
fn installed_old_attach_is_superseded_only_by_a_real_new_registry_winner() {
    run_local_for_bridge_test(|| newer_registry_winner_survives(AttachPause::AfterInstall));
}

#[test]
fn unavailable_identity_stamp_is_a_stamp_error_not_supersession() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = acp::SessionId::new(uuid::Uuid::now_v7().to_string());
        let guard = agent.begin_session_load(&sid);
        // The profile's no-op persistence has a closed sender, not a durable stamp ACK.
        let outcome = spawn_install(
            &agent, &sid, cwd.path(), Some(&guard.rx), CompletionWakePolicy::HumanStopped,
        ).await;
        assert!(matches!(outcome, SessionInstallOutcome::StampFailed));
        assert_eq!(
            outcome.require_current().unwrap_err().data,
            Some(serde_json::json!("session identity stamp failed")),
        );
        assert!(!agent.is_resident(&sid));
        drop(guard);
        assert!(agent.session_registry.root_for_session(&sid).is_none());
        agent.remove_session(&sid);
    });
}
