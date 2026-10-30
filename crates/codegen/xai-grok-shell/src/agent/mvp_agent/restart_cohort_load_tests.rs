use super::*;
use crate::session::completion_delivery::{CompletionEventPolicy, CompletionObligation, CompletionPayloadSource};
use crate::session::persistence::PersistenceMsg;
use xai_grok_tools::computer::types::{TaskKind, TaskSnapshot};
use xai_grok_tools::types::restart_recovery::LeaderRestartRecovery;

async fn enroll(agent: &MvpAgent, sid: &acp::SessionId, task_id: &str) {
    let handle = agent.resident_handle(sid).unwrap();
    let snapshot = TaskSnapshot {
        task_id: task_id.into(), command: "completed cohort work".into(), display_command: None,
        cwd: handle.info.cwd.clone(), start_time: std::time::SystemTime::UNIX_EPOCH,
        end_time: Some(std::time::SystemTime::UNIX_EPOCH), output: task_id.into(),
        output_file: Default::default(), truncated: false, output_total_bytes: task_id.len(),
        exit_code: Some(0), signal: None, completed: true, kind: TaskKind::Bash,
        block_waited: false, explicitly_killed: false, kill_result_delivered: false,
        owner_session_id: Some(sid.to_string()), description: None, is_backgrounded: true,
    };
    let (respond_to, response) = tokio::sync::oneshot::channel();
    handle.persistence_tx.send(PersistenceMsg::EnrollTerminalCompletion {
        source: CompletionPayloadSource::Task(Box::new(snapshot)), text: task_id.into(),
        policy: CompletionEventPolicy::BoundaryOnly, respond_to,
    }).unwrap();
    response.await.unwrap().unwrap();
}

async fn completion_state(agent: &MvpAgent, sid: &acp::SessionId) -> crate::session::completion_delivery::CompletionDeliveryState {
    let handle = agent.resident_handle(sid).unwrap();
    let (respond_to, response) = tokio::sync::oneshot::channel();
    handle.persistence_tx.send(PersistenceMsg::GetCompletionState { respond_to }).unwrap();
    response.await.unwrap().unwrap()
}

#[test]
fn repeated_real_loads_preserve_post_startup_work_after_reactivation_ack() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        agent.start_subagent_coordinator();
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        enroll(&agent, &sid, "old-cohort-task").await;
        drain_persistence(&agent, &sid).await;
        agent.remove_session(&sid);
        agent.cfg.borrow_mut().leader_restart_recovery = LeaderRestartRecovery::stale(chrono::Utc::now());
        for _ in 0..2 {
            agent.load_session_inner(acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf())).await.unwrap();
            let state = completion_state(&agent, &sid).await;
            assert!(!state.restart_admission.permits_recovery());
            assert!(state.obligations.iter().all(|entry| matches!(entry, CompletionObligation::Suppressed { .. })));
            assert!(agent.resident_handle(&sid).unwrap().current_prompt_id.lock().unwrap().is_none());
            drain_persistence(&agent, &sid).await;
            agent.remove_session(&sid);
        }
        agent.load_session_inner(acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf())).await.unwrap();
        agent.resident_handle(&sid).unwrap().admit_new_user_message_after_restart().await.unwrap();
        enroll(&agent, &sid, "new-user-task").await;
        for _ in 0..2 {
            drain_persistence(&agent, &sid).await;
            agent.remove_session(&sid);
            agent.load_session_inner(acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf())).await.unwrap();
            let state = completion_state(&agent, &sid).await;
            assert!(state.restart_admission.permits_recovery());
            assert!(state.obligations.iter().any(|entry| matches!(entry, CompletionObligation::Pending { payload, .. } if payload.text == "new-user-task")));
            assert!(state.obligations.iter().any(|entry| matches!(entry, CompletionObligation::Suppressed { .. })));
            assert!(agent.resident_handle(&sid).unwrap().current_prompt_id.lock().unwrap().is_none());
        }
        agent.remove_session(&sid);
    });
}

#[test]
fn continuing_restart_real_load_preserves_idle_and_pending_boundary_work() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        enroll(&agent, &sid, "fresh-boundary-task").await;
        drain_persistence(&agent, &sid).await;
        agent.remove_session(&sid);
        agent.load_session_inner(acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf())).await.unwrap();
        let state = completion_state(&agent, &sid).await;
        assert!(state.restart_admission.permits_recovery());
        assert!(state.obligations.iter().any(|entry| matches!(entry, CompletionObligation::Pending { payload, .. } if payload.text == "fresh-boundary-task")));
        assert!(agent.resident_handle(&sid).unwrap().current_prompt_id.lock().unwrap().is_none());
        agent.remove_session(&sid);
    });
}
