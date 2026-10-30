use super::*;

#[tokio::test]
async fn stale_restart_cancels_restored_interrupted_and_paused_runs_durably() {
    use crate::session::workflow::tracker::{WorkflowRunStatus, WorkflowTracker};
    let directory = tempfile::tempdir().unwrap();
    let info = SessionInfo { id: agent_client_protocol::SessionId::new("test-session"), cwd: "/tmp".into() };
    let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
    storage.init_session(&info, agent_client_protocol::ModelId::new("test")).await.unwrap();
    let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
    let _persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
    let (manager, _) = WorkflowManager::test_bundle_with_session_dir(Some(directory.path().to_path_buf()));
    let mut manager = manager.lock().await;
    manager.store = WorkflowRunStore::new(Some(directory.path().to_path_buf()), persistence.handle.tx.clone());
    let (gateway, _gateway) = mpsc::unbounded_channel();
    manager.notify = WorkflowNotifySender::new(info.id.clone(), xai_acp_lib::AcpAgentGatewaySender::new(gateway), persistence.handle.tx.clone(), manager.store.clone());
    let statuses = [WorkflowRunStatus::Active, WorkflowRunStatus::Interrupted,
        WorkflowRunStatus::UserPaused, WorkflowRunStatus::BackOffPaused, WorkflowRunStatus::NoProgressPaused,
        WorkflowRunStatus::InfraPaused, WorkflowRunStatus::Blocked, WorkflowRunStatus::BudgetLimited,
        WorkflowRunStatus::Complete, WorkflowRunStatus::Failed, WorkflowRunStatus::Cancelled];
    let mut seed = WorkflowTracker::default();
    let snapshots = statuses.iter().enumerate().map(|(index, status)| {
        let id = format!("wf_restart_{index}");
        let mut state = seed.start_run(id.clone(), id.clone(), "old objective".into(), Vec::new(), Some(10), None);
        state.status = *status;
        state
    }).collect::<Vec<_>>();
    manager.tracker = Arc::new(parking_lot::Mutex::new(WorkflowTracker::from_snapshot(snapshots.clone())));
    for state in &snapshots {
        manager.store.register(&state.run_id, "#{ result: \"old\" };", &serde_json::json!({}), None).unwrap();
        bounded(manager.store.persist_ack(state)).await.unwrap();
    }
    let interrupted = &snapshots[1].run_id;
    assert!(!manager.cancel(interrupted), "ordinary cancellation retains Interrupted semantics");
    assert_eq!(manager.tracker.lock().get(interrupted).unwrap().status, WorkflowRunStatus::Interrupted);
    for state in snapshots {
        let cancels = !matches!(state.status, WorkflowRunStatus::Complete | WorkflowRunStatus::Failed | WorkflowRunStatus::Cancelled);
        assert_eq!(manager.cancel_stale_restart(&state.run_id), cancels, "{:?}", state.status);
        let persisted = bounded(manager.persist_control_result(&state.run_id)).await.unwrap();
        let expected = if cancels { WorkflowRunStatus::Cancelled } else { state.status };
        assert_eq!(persisted.status, expected);
        if cancels {
            assert!(persisted.revision > state.revision);
            assert!(persisted.history.iter().any(|entry| entry.event == "workflow_cancelled"));
        }
        let manifest: crate::session::workflow::store::WorkflowRunManifest = serde_json::from_slice(
            &std::fs::read(directory.path().join("workflows").join(&state.run_id).join("state.json")).unwrap(),
        ).unwrap();
        assert_eq!(manifest.state.status, expected, "durable workflow lifecycle state");
        assert_eq!(manifest.state.revision, persisted.revision);
    }
}
