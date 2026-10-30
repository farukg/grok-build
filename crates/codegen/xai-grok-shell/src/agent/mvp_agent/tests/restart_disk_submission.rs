use super::*;

fn install_model(agent: &crate::agent::mvp_agent::MvpAgent, server: &xai_grok_test_support::MockInferenceServer) {
    let mut model = crate::agent::config::ModelEntry::fallback("test-model", &Default::default());
    model.info.base_url = server.url();
    model.info.api_backend = crate::sampling::ApiBackend::Responses;
    model.api_key = Some("test".into());
    agent.models_manager.insert_test_entry("test-model", model);
}

#[test]
fn disk_childof_submission_continues_exact_child_with_quiescent_parent() {
    xai_test_utils::require_git!();
    run_local_for_bridge_test(|| async {
        let server = xai_grok_test_support::MockInferenceServer::start().await.unwrap();
        server.set_keep_requests(true);
        server.set_response("original native result");
        let original = build_minimal_agent_for_tests();
        install_model(&original, &server);
        let directory = tempfile::tempdir().unwrap();
        let (repository, _remote) = xai_test_utils::git::seed_repo_with_remote(directory.path());
        let parent = super::super::super::new_root_session(&original, &repository).await;
        original.start_subagent_coordinator();
        let id = uuid::Uuid::now_v7().to_string();
        let backend = ChannelBackend::for_coordinator_session(original.subagent_event_tx.clone(), parent.0.to_string());
        let mut request = auto_wake_test_request(&id);
        request.parent_session_id = parent.0.to_string();
        request.prompt = "complete before disk-only restart".into();
        request.run_in_background = false;
        request.await_to_completion = true;
        assert!(bounded(backend.spawn(request, None, None)).await.unwrap().success());
        let source = crate::agent::subagent::durable_resume_source(&id, parent.0.as_ref(), &repository).unwrap();
        let info = crate::session::info::Info { id: acp::SessionId::new(id.clone()), cwd: source.child_cwd };
        super::super::super::drain_persistence(&original, &parent).await;
        original.resident_handle(&parent).unwrap().cmd_tx.send(crate::session::SessionCommand::Shutdown(crate::session::ShutdownKind::Graceful)).unwrap();
        original.remove_session(&parent);
        original.drain_old_session_thread(&parent).await;
        let restarted = build_minimal_agent_for_tests();
        install_model(&restarted, &server);
        let other = super::super::super::new_root_session(&restarted, &repository).await;
        let startup = xai_grok_tools::types::restart_recovery::LeaderRestartRecovery::stale(chrono::Utc::now());
        restarted.cfg.borrow_mut().leader_restart_recovery = startup.clone();
        restarted.start_subagent_coordinator();
        bounded(restarted.load_session_inner(acp::LoadSessionRequest::new(parent.clone(), repository.clone()))).await.unwrap();
        assert!(restarted.session_host(&info.id).is_none(), "fresh coordinator has no child route");
        assert!(bounded(restarted.resume_subagent(parent.0.as_ref(), &id, "generic resume".into())).await.is_err());
        let foreign = bounded(acp::Agent::prompt(&restarted, acp::PromptRequest::new(info.id.clone(), vec![acp::ContentBlock::from(WAKE_TEXT)])
            .meta(serde_json::json!({"childOf": other.0}).as_object().cloned()))).await;
        assert!(foreign.is_err(), "disk metadata cannot be borrowed by another parent");
        let mut response = server.expect_response_blocked("disk continuation", InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Responses, WAKE_TEXT),
            ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact("disk continuation result", "test-model")));
        let receipt = bounded(acp::Agent::prompt(&restarted, acp::PromptRequest::new(info.id.clone(), vec![acp::ContentBlock::from(WAKE_TEXT)])
            .meta(serde_json::json!({"childOf": parent.0}).as_object().cloned()))).await.unwrap();
        assert_eq!(receipt.meta.unwrap()["childResume"], serde_json::json!({"kind":"resumed","sourceId": id}));
        bounded(response.wait_blocked()).await;
        let requests = server.requests().into_iter().filter(|entry| entry.body.as_ref().is_some_and(|body| body.to_string().contains(WAKE_TEXT))).collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        let continuation = requests[0].header("x-grok-session-id").unwrap().to_owned();
        assert_ne!(continuation, id);
        assert_ne!(continuation, parent.0.as_ref());
        assert!(!startup.session(parent.0.as_ref()).admission().permits_recovery());
        assert!(matches!(startup.session(&id).admission(), xai_grok_tools::types::restart_recovery::RestartAdmission::Reactivated { .. }));
        response.release();
        let backend = ChannelBackend::for_coordinator_session(restarted.subagent_event_tx.clone(), parent.0.to_string());
        let completed = bounded(backend.query(&continuation, true, Some(30_000), None)).await.expect("query acknowledgement").unwrap();
        assert!(matches!(completed.status, SubagentSnapshotStatus::Completed { ref output, .. } if output.contains("disk continuation result")));
        let storage = JsonlStorageAdapter::with_root(crate::util::grok_home::grok_home());
        let old = storage.load_session(&info).await.unwrap();
        assert!(!old.chat_history.iter().any(|item| item.text_content().contains(WAKE_TEXT)));
        assert!(!startup.session(parent.0.as_ref()).admission().permits_recovery());
        response.assert_satisfied();
    });
}
