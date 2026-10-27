use super::{build_minimal_agent_for_tests, new_root_session, run_local_for_bridge_test};
use crate::extensions::context_policy::{
    CONTEXT_POLICY_CHANGED, ContextPolicyAction, ContextPolicyFailure, ContextPolicyNotification,
    ContextPolicyRequest, ContextPolicyResponse,
};
use crate::session::helpers::context_policy_store::ContextPolicyStore;
use agent_client_protocol as acp;
use xai_grok_sampling_types::{ContextCategory, ContextPolicyEdit, ContextPolicyMutation, ContextScope, ContextSwitch};

async fn request(agent: &super::MvpAgent, id: &acp::SessionId, action: ContextPolicyAction) -> ContextPolicyResponse {
    let params = serde_json::value::to_raw_value(&ContextPolicyRequest { session_id: id.0.to_string(), action }).unwrap();
    let response = crate::extensions::context_policy::handle(agent,
        &acp::ExtRequest::new("x.ai/session/context_policy", params.into())).await.unwrap();
    serde_json::from_str(response.0.get()).unwrap()
}

async fn version(agent: &super::MvpAgent, id: &acp::SessionId) -> xai_grok_sampling_types::ContextPolicyVersion {
    let ContextPolicyResponse::Confirmed { report } = request(agent, id, ContextPolicyAction::Get).await else { panic!("authoritative read"); };
    report.selection.unwrap().version
}

#[test]
fn global_edits_reach_resident_and_future_actors_keep_old_requests_and_push_confirmed_state() {
    run_local_for_bridge_test(|| async {
        let root = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let mut agent = build_minimal_agent_for_tests();
        agent.context_policy_authority = std::sync::Arc::new(tokio::sync::RwLock::new(ContextPolicyStore::new(root.path().to_owned())));
        let (gateway, mut notifications) = tokio::sync::mpsc::unbounded_channel();
        agent.gateway = xai_acp_lib::AcpAgentGatewaySender::new(gateway);
        let id = new_root_session(&agent, cwd.path()).await;
        let resident = agent.new_session_inner(acp::NewSessionRequest::new(cwd.path().to_owned())).await.unwrap().session_id;
        let handle = agent.resident_handle(&id).unwrap();
        let old = handle.chat_state_handle.build_request(Vec::new(), None, false, None,
            id.0.to_string(), "before-global".into()).await.unwrap();
        assert!(old.context_policy.includes(ContextCategory::Reasoning));
        let expected = version(&agent, &id).await;
        let response = request(&agent, &id, ContextPolicyAction::Edit { edit: ContextPolicyEdit {
            expected,
            mutation: ContextPolicyMutation::SetCategories {
                categories: vec![ContextCategory::Reasoning, ContextCategory::Memory].try_into().unwrap(),
                value: ContextSwitch::Excluded, target: ContextScope::Global,
            },
        }}).await;
        let ContextPolicyResponse::Confirmed { report } = response else { panic!("durable global admission"); };
        assert!(!report.policy.includes(ContextCategory::Reasoning));
        assert!(old.context_policy.includes(ContextCategory::Reasoning));
        let next = handle.chat_state_handle.build_request(Vec::new(), None, false, None,
            id.0.to_string(), "after-global".into()).await.unwrap();
        assert!(!next.context_policy.includes(ContextCategory::Reasoning));
        assert!(!next.context_policy.includes(ContextCategory::Memory));
        let resident_policy = agent.resident_handle(&resident).unwrap().chat_state_handle.get_context_policy().await.unwrap();
        assert_eq!(resident_policy, report.policy);
        let following = agent.new_session_inner(acp::NewSessionRequest::new(cwd.path().to_owned())).await.unwrap().session_id;
        let ContextPolicyResponse::Confirmed { report: future } = request(&agent, &following, ContextPolicyAction::Get).await else { panic!("future read"); };
        assert_eq!(future.policy, report.policy);
        assert_eq!(future.selection.unwrap().scope, ContextScope::Global);
        let mut push_seen = false;
        while let Ok(message) = notifications.try_recv() {
            if let xai_acp_lib::AcpClientMessage::ExtNotification(args) = message
                && args.request.method.as_ref() == CONTEXT_POLICY_CHANGED
            {
                let push: ContextPolicyNotification = serde_json::from_str(args.request.params.get()).unwrap();
                if push.session_id == id.0.as_ref() {
                    let ContextPolicyResponse::Confirmed { report: pushed } = push.response else { panic!("confirmed push"); };
                    if pushed.selection.as_ref().unwrap().version == report.selection.as_ref().unwrap().version {
                        assert_eq!(pushed.policy, report.policy);
                        push_seen = true;
                    }
                }
            }
        }
        assert!(push_seen, "attached clients receive the actual applied policy");
        let mut restarted = ContextPolicyStore::new(root.path().to_owned());
        restarted.load().unwrap();
        assert_eq!(restarted.effective("future-after-restart").unwrap(), report.policy);
        agent.request_session_shutdown(&id);
        agent.request_session_shutdown(&following);
        agent.request_session_shutdown(&resident);
    });
}

#[test]
fn stale_or_replaced_origin_confirmation_has_no_durable_or_actor_effect() {
    run_local_for_bridge_test(|| async {
        let root = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let mut agent = build_minimal_agent_for_tests();
        agent.context_policy_authority = std::sync::Arc::new(tokio::sync::RwLock::new(ContextPolicyStore::new(root.path().to_owned())));
        let id = new_root_session(&agent, cwd.path()).await;
        let stale = version(&agent, &id).await;
        let mutation = ContextPolicyMutation::SetCategories {
            categories: vec![ContextCategory::Reasoning].try_into().unwrap(), value: ContextSwitch::Excluded, target: ContextScope::Session,
        };
        assert!(matches!(request(&agent, &id, ContextPolicyAction::Edit { edit: ContextPolicyEdit { expected: stale.clone(), mutation: mutation.clone() }}).await,
            ContextPolicyResponse::Confirmed { .. }));
        let saved = std::fs::read(root.path().join("context_policy.toml")).unwrap();
        assert!(matches!(request(&agent, &id, ContextPolicyAction::Edit { edit: ContextPolicyEdit {
            expected: stale, mutation: ContextPolicyMutation::SelectScope { scope: ContextScope::Global },
        }}).await, ContextPolicyResponse::Rejected { failure: ContextPolicyFailure::Stale, current: Some(_) }));
        let mut origin = version(&agent, &id).await;
        origin.incarnation = "replaced-actor".into();
        assert!(matches!(request(&agent, &id, ContextPolicyAction::Edit { edit: ContextPolicyEdit { expected: origin, mutation }}).await,
            ContextPolicyResponse::Rejected { failure: ContextPolicyFailure::OriginChanged, current: Some(_) }));
        assert_eq!(std::fs::read(root.path().join("context_policy.toml")).unwrap(), saved);
        assert!(!agent.resident_handle(&id).unwrap().chat_state_handle.get_context_policy().await.unwrap().includes(ContextCategory::Reasoning));
        agent.request_session_shutdown(&id);
    });
}

#[test]
fn a_failed_actor_does_not_roll_back_the_durable_global_policy_or_successful_actors() {
    run_local_for_bridge_test(|| async {
        let root = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let mut agent = build_minimal_agent_for_tests();
        agent.context_policy_authority = std::sync::Arc::new(tokio::sync::RwLock::new(ContextPolicyStore::new(root.path().to_owned())));
        let origin = new_root_session(&agent, cwd.path()).await;
        let failed = agent.new_session_inner(acp::NewSessionRequest::new(cwd.path().to_owned())).await.unwrap().session_id;
        agent.with_resident_mut(&failed, |handle| handle.chat_state_handle = xai_chat_state::ChatStateHandle::noop()).unwrap();
        let expected = version(&agent, &origin).await;
        let response = request(&agent, &origin, ContextPolicyAction::Edit { edit: ContextPolicyEdit {
            expected,
            mutation: ContextPolicyMutation::SetCategories {
                categories: vec![ContextCategory::Reasoning].try_into().unwrap(), value: ContextSwitch::Excluded, target: ContextScope::Global,
            },
        }}).await;
        let ContextPolicyResponse::Rejected { failure: ContextPolicyFailure::Apply { failures, .. }, current: Some(current) } = response else { panic!("partial admission is not success or rollback"); };
        assert!(failures.iter().any(|failure| failure.session_id == failed.0.as_ref()
            && matches!(failure.cause, crate::extensions::context_policy::ContextAdmissionFailure::MailboxClosed)));
        assert!(!current.policy.includes(ContextCategory::Reasoning));
        assert!(!agent.resident_handle(&origin).unwrap().chat_state_handle.get_context_policy().await.unwrap().includes(ContextCategory::Reasoning));
        let mut restored = ContextPolicyStore::new(root.path().to_owned());
        restored.load().unwrap();
        assert!(!restored.effective("future").unwrap().includes(ContextCategory::Reasoning));
        agent.request_session_shutdown(&origin);
        agent.request_session_shutdown(&failed);
    });
}

#[test]
fn a_storage_error_reports_the_actual_unchanged_actor_not_the_requested_policy() {
    run_local_for_bridge_test(|| async {
        let root = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let mut agent = build_minimal_agent_for_tests();
        agent.context_policy_authority = std::sync::Arc::new(tokio::sync::RwLock::new(ContextPolicyStore::new(root.path().to_owned())));
        let origin = new_root_session(&agent, cwd.path()).await;
        let expected = version(&agent, &origin).await;
        std::fs::create_dir(root.path().join("context_policy.toml")).unwrap();
        let response = request(&agent, &origin, ContextPolicyAction::Edit { edit: ContextPolicyEdit {
            expected,
            mutation: ContextPolicyMutation::SetCategories {
                categories: vec![ContextCategory::Reasoning].try_into().unwrap(), value: ContextSwitch::Excluded, target: ContextScope::Global,
            },
        }}).await;
        let ContextPolicyResponse::Rejected { failure: ContextPolicyFailure::StorageIo { .. }, current: Some(current) } = response else { panic!("typed durable failure with actual state"); };
        assert!(current.policy.includes(ContextCategory::Reasoning));
        assert_eq!(current.selection.unwrap().version.revision, 0);
        assert!(agent.resident_handle(&origin).unwrap().chat_state_handle.get_context_policy().await.unwrap().includes(ContextCategory::Reasoning));
        agent.request_session_shutdown(&origin);
    });
}

#[test]
fn an_empty_bulk_target_is_rejected_at_the_wire_boundary_without_any_change() {
    run_local_for_bridge_test(|| async {
        let root = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let mut agent = build_minimal_agent_for_tests();
        agent.context_policy_authority = std::sync::Arc::new(tokio::sync::RwLock::new(ContextPolicyStore::new(root.path().to_owned())));
        let id = new_root_session(&agent, cwd.path()).await;
        let expected = version(&agent, &id).await;
        let raw = serde_json::value::to_raw_value(&serde_json::json!({
            "sessionId": id.0.as_ref(), "action": "edit",
            "edit": { "expected": expected, "mutation": {
                "change": "set_categories", "categories": [], "value": "excluded", "target": "global"
            }}
        })).unwrap();
        assert!(crate::extensions::context_policy::handle(&agent,
            &acp::ExtRequest::new("x.ai/session/context_policy", raw.into())).await.is_err());
        assert!(!root.path().join("context_policy.toml").exists());
        assert!(agent.resident_handle(&id).unwrap().chat_state_handle.get_context_policy().await.unwrap().is_unrestricted());
        agent.request_session_shutdown(&id);
    });
}
