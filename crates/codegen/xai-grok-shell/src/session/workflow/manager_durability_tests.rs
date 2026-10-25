use super::*;
use crate::session::persistence::PersistenceMsg;
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter};
use crate::session::{SessionCommand, SessionInfo};

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(15), future).await.expect("workflow durability rendezvous timed out")
}

#[tokio::test]
async fn actual_workflow_manager_holds_outcome_and_wake_until_manifest_enrollment_ack() {
    for lost_ack in [None, Some(false), Some(true)] {
    let directory = tempfile::tempdir().unwrap();
    let info = SessionInfo { id: agent_client_protocol::SessionId::new("test-session"), cwd: "/tmp".into() };
    let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
    storage.init_session(&info, agent_client_protocol::ModelId::new("test")).await.unwrap();
    let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
    let persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
    let (proxy, mut requests) = mpsc::unbounded_channel();
    let (committed, mut gates) = mpsc::unbounded_channel();
    let real = persistence.handle.tx.clone();
    let proxy_task = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            match request {
                PersistenceMsg::WorkflowRunStateAndAck { manifest, respond_to } => {
                    let (reply, response) = oneshot::channel();
                    real.send(PersistenceMsg::WorkflowRunStateAndAck { manifest: manifest.clone(), respond_to: reply }).unwrap();
                    let durable_result = response.await.unwrap();
                    committed.send((manifest, respond_to, durable_result)).unwrap();
                }
                other => { real.send(other).unwrap(); }
            }
        }
    }));
    let (manager, _) = WorkflowManager::test_bundle_with_session_dir(Some(directory.path().to_path_buf()));
    let (session_tx, mut session_rx) = mpsc::unbounded_channel();
    let (gateway_tx, _gateway_rx) = mpsc::unbounded_channel();
    {
        let mut manager = manager.lock().await;
        manager.store = WorkflowRunStore::new(Some(directory.path().to_path_buf()), proxy.clone());
        manager.notify = WorkflowNotifySender::new(info.id.clone(), xai_acp_lib::AcpAgentGatewaySender::new(gateway_tx), proxy, manager.store.clone());
        manager.session_cmd_tx = session_tx;
    }
    let resolved = super::super::registry::resolve_inline("let meta = #{ name: \"ack-barrier\", description: \"real workflow outcome\" };\n#{ result: \"finished\" };".into()).unwrap();
    let (run_id, mut outcome) = manager.lock().await.launch(resolved, LaunchSpec {
        objective: "prove durable publication".into(), args: serde_json::json!({}), agent_budget: Some(10), effort: None, resume_run_id: None,
    }).unwrap();
    let (manifest, ack, result) = bounded(gates.recv()).await.unwrap();
    result.unwrap();
    assert_eq!(manifest.state.run_id, run_id);
    assert!(manifest.state.status.is_completion_reportable());
    assert!(matches!(outcome.try_recv(), Err(oneshot::error::TryRecvError::Empty)), "engine completion cannot overtake the manifest/obligation ACK");
    assert!(session_rx.try_recv().is_err(), "terminal wake cannot precede enrollment acknowledgement");
    let on_disk: super::super::store::WorkflowRunManifest = serde_json::from_slice(&std::fs::read(directory.path().join("workflows").join(&run_id).join("state.json")).unwrap()).unwrap();
    assert_eq!(on_disk.state.revision, manifest.state.revision);
    assert_eq!(on_disk.state.status, manifest.state.status);
    let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    let recovered = reopened.recover_subagent_completions(&info).await.unwrap().completion_delivery;
    let identity: xai_chat_state::completion::CompletionIdentity = xai_chat_state::completion::TerminalCompletionId::Workflow { run_id: run_id.clone(), revision: manifest.state.revision }.into();
    assert!(recovered.obligations.iter().any(|obligation| obligation.id() == identity));
    assert!(recovered.receipts.is_empty());
    assert!(matches!(outcome.try_recv(), Err(oneshot::error::TryRecvError::Empty)), "reopening committed storage does not acknowledge the old producer");
    let expected_revision = match lost_ack {
        None => { ack.send(Ok(())).unwrap(); manifest.state.revision }
        Some(lost) => {
            if lost { drop(ack); } else { ack.send(Err(std::io::Error::other("injected manifest ACK failure"))).unwrap(); }
            let (interrupted, interrupted_ack, committed) = bounded(gates.recv()).await.unwrap();
            committed.unwrap();
            assert!(matches!(outcome.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            assert!(session_rx.try_recv().is_err(), "failed or lost original ACK cannot release a terminal wake");
            assert_eq!(interrupted.state.status, WorkflowRunStatus::Interrupted);
            assert!(interrupted.state.revision > manifest.state.revision);
            interrupted_ack.send(Ok(())).unwrap();
            interrupted.state.revision
        }
    };
    let terminal = bounded(outcome).await.unwrap();
    match lost_ack {
        None => assert!(matches!(terminal, WorkflowOutcome::Completed { .. })),
        Some(_) => assert!(matches!(terminal, WorkflowOutcome::Failed { .. })),
    }
    let SessionCommand::WorkflowCompletionTurn { run_id: published, revision } = bounded(session_rx.recv()).await.unwrap() else { panic!("expected existing workflow completion transport") };
    assert_eq!(published, run_id);
    assert_eq!(revision, expected_revision);
    assert!(session_rx.try_recv().is_err(), "one terminal revision publishes one wake");
    drop(proxy_task);
    drop(persistence_task);
    }
}
