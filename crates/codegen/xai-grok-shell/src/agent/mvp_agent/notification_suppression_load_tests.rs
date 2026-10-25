use super::*;
use crate::session::completion_delivery::{CompletionEventPolicy, CompletionObligation, CompletionPayloadSource, CompletionSuppression};
use crate::session::persistence::PersistenceMsg;
use xai_chat_state::completion::{CompletionIdentity, CompletionTaskKind, TerminalCompletionId};
use xai_grok_tools::computer::types::{TaskKind, TaskSnapshot};

#[test]
fn real_boundary_only_producer_enrollment_survives_actual_cold_load_before_policy_admission() {
    use crate::session::commands::SessionCommand;
    use crate::tools::notification_bridge::{spawn_notification_bridge, tests::make_test_config_full_raw};
    use xai_grok_tools::computer::local::terminal::LocalTerminalBackend;
    use xai_grok_tools::computer::types::{TerminalBackend, TerminalRunRequest};
    run_local_for_bridge_test(|| async {
        for kind in [TaskKind::Bash, TaskKind::Monitor] {
            let agent = build_minimal_agent_for_tests();
            let cwd = tempfile::tempdir().unwrap();
            let sid = new_root_session(&agent, cwd.path()).await;
            let handle = agent.resident_handle(&sid).unwrap();
            let (mut config, mut publications, _unused_persistence, mut admissions) = make_test_config_full_raw();
            config.session_id = sid.clone();
            config.persistence = crate::session::persistence::PersistenceHandle::from_sender_for_test(handle.persistence_tx.clone());
            config.auto_wake_enabled = false;
            let terminal = LocalTerminalBackend::new_local(Default::default());
            let background = terminal.run_background(TerminalRunRequest {
                command: "printf cold-load-boundary-evidence".into(), working_directory: cwd.path().to_path_buf(), env: Default::default(),
                timeout: std::time::Duration::from_secs(10), output_byte_limit: 4096,
                output_file: cwd.path().join("terminal-output"), notification_handle: spawn_notification_bridge(config),
                tool_call_id: format!("cold-load-{kind:?}"), display_command: None, auto_background_on_timeout: false,
                foreground_block_budget: None, kind, owner_session_id: Some(sid.to_string()), description: None,
            }).await.unwrap();
            let policy_ack = tokio::time::timeout(std::time::Duration::from_secs(15), async {
                loop {
                    if let SessionCommand::ClassifyTerminalNotification { respond_to, .. } = admissions.recv().await.unwrap() { break respond_to; }
                }
            }).await.unwrap();
            let task_kind = match kind { TaskKind::Bash => CompletionTaskKind::Bash, TaskKind::Monitor => CompletionTaskKind::Monitor };
            let expected: CompletionIdentity = TerminalCompletionId::Task { owner_session_id: sid.to_string(), task_id: background.task_id.clone(), task_kind }.into();
            let (respond_to, response) = tokio::sync::oneshot::channel();
            handle.persistence_tx.send(PersistenceMsg::GetCompletionState { respond_to }).unwrap();
            let enrolled = response.await.unwrap().unwrap();
            assert!(enrolled.receipts.is_empty());
            assert!(matches!(&enrolled.obligations[..], [CompletionObligation::Pending { id, payload }] if id == &expected && matches!(payload.policy, CompletionEventPolicy::BoundaryOnly)));
            while let Ok(message) = publications.try_recv() {
                if let xai_acp_lib::AcpClientMessage::ExtNotification(message) = message {
                    assert_ne!(message.request.method.as_ref(), "x.ai/task_completed", "producer must not publish before policy ACK");
                }
            }
            drain_persistence(&agent, &sid).await;
            drop(handle);
            drop(terminal);
            drop(admissions);
            drop(policy_ack);
            agent.remove_session(&sid);
            assert!(!agent.is_resident(&sid));
            agent.load_session_inner(acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf())).await.unwrap();
            let restored = agent.resident_handle(&sid).unwrap();
            let (respond_to, response) = tokio::sync::oneshot::channel();
            restored.cmd_tx.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
            response.await.unwrap();
            assert!(restored.current_prompt_id.lock().unwrap().is_none());
            let history = restored.chat_state_handle.snapshot().await.unwrap().conversation;
            assert!(history.iter().all(|item| !item.text_content().contains("cold-load-boundary-evidence") && !item.text_content().contains(&background.task_id)));
            let (respond_to, response) = tokio::sync::oneshot::channel();
            restored.persistence_tx.send(PersistenceMsg::GetCompletionState { respond_to }).unwrap();
            let recovered = response.await.unwrap().unwrap();
            assert!(recovered.receipts.is_empty());
            let [CompletionObligation::Pending { id, payload }] = &recovered.obligations[..] else { panic!("BoundaryOnly producer must remain pending after cold load") };
            assert_eq!(id, &expected);
            assert!(matches!(payload.policy, CompletionEventPolicy::BoundaryOnly));
            let CompletionPayloadSource::Task(snapshot) = &payload.source else { panic!("cold load must retain the real producer snapshot") };
            assert_eq!(snapshot.task_id, background.task_id);
            assert_eq!(snapshot.owner_session_id.as_deref(), Some(sid.0.as_ref()));
            assert_eq!(snapshot.kind, kind);
            assert_eq!(snapshot.command, "printf cold-load-boundary-evidence");
            assert_eq!(snapshot.cwd, cwd.path().to_string_lossy().into_owned());
            assert_eq!(snapshot.output, "cold-load-boundary-evidence");
            assert_eq!(snapshot.output_file, cwd.path().join("terminal-output"));
            assert_eq!(snapshot.exit_code, Some(0));
            assert!(snapshot.completed && snapshot.is_backgrounded && !snapshot.block_waited);
            agent.remove_session(&sid);
        }
    });
}

#[test]
fn actual_cold_load_keeps_suppressed_terminal_silent_and_boundary_peer_pending() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        let handle = agent.resident_handle(&sid).unwrap();
        let mut suppressed = Vec::new();
        for (task_id, kind, policy) in [
            ("load-silent-bash", TaskKind::Bash, CompletionEventPolicy::Automatic),
            ("load-silent-monitor", TaskKind::Monitor, CompletionEventPolicy::Automatic),
            ("load-boundary-peer", TaskKind::Bash, CompletionEventPolicy::BoundaryOnly),
        ] {
            let snapshot = TaskSnapshot {
                task_id: task_id.into(), command: "finished".into(), display_command: None, cwd: cwd.path().to_string_lossy().into_owned(),
                start_time: std::time::SystemTime::UNIX_EPOCH, end_time: Some(std::time::SystemTime::UNIX_EPOCH),
                output: "load terminal evidence".into(), output_file: Default::default(), truncated: false, exit_code: Some(0), signal: None,
                completed: true, kind, block_waited: false, explicitly_killed: false, kill_result_delivered: false,
                owner_session_id: Some(sid.to_string()), description: None, is_backgrounded: true, output_total_bytes: "load terminal evidence".len(),
            };
            let (respond_to, response) = tokio::sync::oneshot::channel();
            handle.persistence_tx.send(PersistenceMsg::EnrollTerminalCompletion {
                source: CompletionPayloadSource::Task(Box::new(snapshot)), text: task_id.into(), policy, respond_to,
            }).unwrap();
            response.await.unwrap().unwrap();
            if matches!(policy, CompletionEventPolicy::Automatic) {
                let task_kind = match kind { TaskKind::Bash => CompletionTaskKind::Bash, TaskKind::Monitor => CompletionTaskKind::Monitor };
                suppressed.push(CompletionIdentity::from(TerminalCompletionId::Task { owner_session_id: sid.to_string(), task_id: task_id.into(), task_kind }));
            }
        }
        let (respond_to, response) = tokio::sync::oneshot::channel();
        handle.persistence_tx.send(PersistenceMsg::SuppressTerminalCompletions {
            ids: suppressed.clone(), reason: CompletionSuppression::GoalNotificationPolicy, respond_to,
        }).unwrap();
        response.await.unwrap().unwrap();
        drain_persistence(&agent, &sid).await;
        drop(handle);
        agent.remove_session(&sid);
        assert!(!agent.is_resident(&sid));
        agent.load_session_inner(acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf())).await.unwrap();
        let restored = agent.resident_handle(&sid).unwrap();
        let (respond_to, response) = tokio::sync::oneshot::channel();
        restored.cmd_tx.send(crate::session::commands::SessionCommand::GetToolOverrides { respond_to }).unwrap();
        response.await.unwrap();
        assert!(restored.current_prompt_id.lock().unwrap().is_none(), "load must not start a suppressed or boundary-only caller");
        let history = restored.chat_state_handle.snapshot().await.unwrap().conversation;
        assert!(history.iter().all(|item| !item.text_content().contains("load-silent-") && !item.text_content().contains("load-boundary-peer")));
        let (respond_to, response) = tokio::sync::oneshot::channel();
        restored.persistence_tx.send(PersistenceMsg::GetCompletionState { respond_to }).unwrap();
        let state = response.await.unwrap().unwrap();
        assert!(state.receipts.is_empty());
        for id in suppressed {
            assert!(state.obligations.iter().any(|obligation| matches!(obligation, CompletionObligation::Suppressed { id: restored, reason: CompletionSuppression::GoalNotificationPolicy, .. } if restored == &id)));
        }
        assert!(state.obligations.iter().any(|obligation| matches!(obligation, CompletionObligation::Pending { id: CompletionIdentity::V2 { terminal: TerminalCompletionId::Task { task_id, .. }, .. }, payload } if task_id == "load-boundary-peer" && matches!(payload.policy, CompletionEventPolicy::BoundaryOnly))));
        agent.remove_session(&sid);
    });
}
