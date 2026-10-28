use super::*;
use crate::agent::subagent::completion_test_support::auto_wake_test_request;
use crate::session::delivery_receiver::{
    DeliveryExecution, DeliveryProvenance, ParentMessageOperation, ParentMessageSource,
};
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter};
use xai_grok_sampling_types::conversation::{ConversationItem, SyntheticReason};
use xai_grok_test_support::{InferenceEndpoint, InferenceRequestMatcher, ScriptedResponse};
use xai_grok_tools::implementations::grok_build::task::backend::{ChannelBackend, SubagentBackend};
use xai_grok_tools::implementations::grok_build::task::types::{
    ActiveAgentMessageOutcome, ActiveAgentMessageRequest, ActiveAgentMessageSource,
    SubagentSnapshotStatus,
};

const WAKE_TEXT: &str = "native-wake-exact-bootstrap-7f23";

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(30), future)
        .await
        .expect("native child lifecycle must settle")
}

#[test]
fn completed_native_worker_admits_agent_bootstrap_once() {
    xai_test_utils::require_git!();
    run_local_for_bridge_test(|| successful_wake(ActiveAgentMessageSource::Agent));
}

#[test]
fn completed_native_worker_admits_human_bootstrap_once() {
    xai_test_utils::require_git!();
    run_local_for_bridge_test(|| successful_wake(ActiveAgentMessageSource::Human));
}

async fn successful_wake(source: ActiveAgentMessageSource) {
    let server = xai_grok_test_support::MockInferenceServer::start()
        .await
        .unwrap();
    server.set_response("original native result");
    server.set_keep_requests(true);
    let agent = build_minimal_agent_for_tests();
    agent
        .cfg
        .borrow_mut()
        .feature_values
        .insert(crate::agent::config::Feature::ActiveAgentMessages, true);
    let mut model = crate::agent::config::ModelEntry::fallback("test-model", &Default::default());
    model.info.base_url = server.url();
    model.info.api_backend = crate::sampling::ApiBackend::Responses;
    model.api_key = Some("test".into());
    agent.models_manager.insert_test_entry("test-model", model);
    let directory = tempfile::tempdir().unwrap();
    let (repository, _remote) = xai_test_utils::git::seed_repo_with_remote(directory.path());
    let parent = super::super::new_root_session(&agent, &repository).await;
    agent.start_subagent_coordinator();
    let id = uuid::Uuid::now_v7().to_string();
    let backend = ChannelBackend::for_coordinator_session(
        agent.subagent_event_tx.clone(),
        parent.0.to_string(),
    );
    let mut request = auto_wake_test_request(&id);
    request.parent_session_id = parent.0.to_string();
    request.prompt = "Complete the original native task.".into();
    request.run_in_background = false;
    request.await_to_completion = true;
    let prior = bounded(backend.spawn(request, None, None)).await.unwrap();
    assert!(prior.success(), "{prior:?}");
    assert!(prior.output.contains("original native result"));
    let child = acp::SessionId::new(id.clone());
    assert!(agent.resident_handle(&child).is_none());
    let persisted =
        crate::agent::subagent::durable_resume_source(&id, parent.0.as_ref(), &repository).unwrap();
    let info = crate::session::info::Info {
        id: child.clone(),
        cwd: persisted.child_cwd,
    };
    let storage = JsonlStorageAdapter::with_root(crate::util::grok_home::grok_home());
    let before = storage.load_summary(&info).await.unwrap();
    assert!(before.is_subagent_kind());
    assert!(
        before.is_hidden(),
        "child history visibility must not be rewritten to admit a wake"
    );
    assert_eq!(
        before.hidden, None,
        "ordinary child history hiding is not an explicit retirement"
    );
    let mut next = server.expect_response_blocked(
        "native wake bootstrap",
        InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Responses, WAKE_TEXT),
        ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact(
            "resumed native result",
            "test-model",
        )),
    );
    let message = match source {
        ActiveAgentMessageSource::Agent => {
            ActiveAgentMessageRequest::try_new(&id, WAKE_TEXT).unwrap()
        }
        ActiveAgentMessageSource::Human => {
            let Some(crate::agent::mvp_agent::SessionHost::Child(ChildHost {
                reach:
                    ChildReach::Addressed {
                        address,
                        residence: ChildResidence::Finished,
                    },
                ..
            })) = agent.session_host(&child)
            else {
                panic!("completed child retains its human route");
            };
            ActiveAgentMessageRequest::try_new_from_human(
                address.as_str(), WAKE_TEXT,
                xai_grok_tools::implementations::grok_build::task::types::ActiveAgentMessageOperation::Queue,
            ).unwrap()
        }
    };
    let accepted = bounded(backend.send_active_message(message)).await;
    let ActiveAgentMessageOutcome::Accepted { message_id } = accepted else {
        panic!("native wake did not commit: {accepted:?}");
    };
    assert!(
        uuid::Uuid::parse_str(&message_id).is_ok(),
        "receipt uses the bare message identity"
    );
    let delivery_id = format!("parent-message/{message_id}");
    let admitted = storage.load_summary(&info).await.unwrap();
    let record = admitted
        .completion_delivery
        .receiver
        .records
        .get(&delivery_id)
        .expect("Accepted requires a durable parent admission");
    let DeliveryProvenance::ParentMessage { admission } = &record.provenance else {
        panic!("native bootstrap must retain parent provenance");
    };
    assert_eq!(admission.message_id, message_id);
    assert_eq!(admission.prompt_id, format!("parent-message-{message_id}"));
    assert_eq!(admission.sender_session_id, parent.0.as_ref());
    assert_eq!(admission.text, WAKE_TEXT);
    assert_eq!(admission.effective, ParentMessageOperation::Queue);
    let expected_source = match source {
        ActiveAgentMessageSource::Agent => ParentMessageSource::Agent,
        ActiveAgentMessageSource::Human => ParentMessageSource::Human,
    };
    assert_eq!(admission.source, expected_source);
    use sha2::{Digest, Sha256};
    assert_eq!(
        record.digest,
        format!("{:x}", Sha256::digest(WAKE_TEXT.as_bytes()))
    );
    bounded(next.wait_blocked()).await;
    let requests = server.request_bodies();
    let resumed = requests
        .iter()
        .filter(|body| body.to_string().contains(WAKE_TEXT))
        .collect::<Vec<_>>();
    assert_eq!(
        resumed.len(),
        1,
        "one real model input, not Prompt plus ParentMessage"
    );
    assert_eq!(resumed[0].to_string().matches(WAKE_TEXT).count(), 1);
    let loaded = storage.load_session(&info).await.unwrap();
    let expected_reason = match source {
        ActiveAgentMessageSource::Agent => SyntheticReason::AgentMessage,
        ActiveAgentMessageSource::Human => SyntheticReason::ParentHumanMessage,
    };
    let bootstraps = loaded
        .chat_history
        .iter()
        .filter(|item| {
            matches!(item, ConversationItem::User(user) if user.synthetic_reason == expected_reason
            && serde_json::to_string(user).unwrap().contains(WAKE_TEXT))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        bootstraps.len(),
        1,
        "the source-specific input is durable once"
    );
    let history = std::fs::read_to_string(
        crate::session::persistence::session_dir(&info).join("chat_history.jsonl"),
    )
    .unwrap();
    let bindings = history
        .lines()
        .filter_map(|line| {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            row.get("_session_delivery").map(|binding| {
                serde_json::from_value::<xai_chat_state::DeliveryBinding>(binding.clone()).unwrap()
            })
        })
        .filter(|binding| binding.delivery_id == delivery_id)
        .collect::<Vec<_>>();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].payload_digest, record.digest);
    assert_eq!(bindings[0].turn_id, admission.prompt_id);
    assert!(
        matches!(&bindings[0].parent_task, xai_chat_state::ParentTask::Instance { id } if !id.is_empty())
    );
    next.release();
    let completed = bounded(backend.query(&id, true, Some(30_000), None))
        .await
        .unwrap();
    assert!(
        matches!(completed.status, SubagentSnapshotStatus::Completed { ref output, .. }
        if output.contains("resumed native result"))
    );
    let terminal = storage.load_summary(&info).await.unwrap();
    assert!(matches!(
        terminal.completion_delivery.receiver.records[&delivery_id].execution,
        DeliveryExecution::Succeeded
    ));
    assert!(terminal.next_trace_turn > before.next_trace_turn);
    next.assert_satisfied();
    let sampled = server.request_count();
    let reopened = bounded(agent.management_session(&child)).await.unwrap();
    let conversation = reopened.handle.chat_state_handle.get_conversation().await;
    assert_eq!(
        serde_json::to_string(&conversation)
            .unwrap()
            .matches(WAKE_TEXT)
            .count(),
        1
    );
    assert_eq!(
        server.request_count(),
        sampled,
        "reopening a projected terminal input must not replay it"
    );
    let recovered = storage.load_summary(&info).await.unwrap();
    assert!(matches!(
        recovered.completion_delivery.receiver.records[&delivery_id].execution,
        DeliveryExecution::Succeeded
    ));
    let reference = recovered.completion_delivery.receiver.records[&delivery_id]
        .parent_recovery_ref()
        .unwrap();
    use xai_grok_tools::implementations::grok_build::task::native_recovery::{
        MessageRecoveryAction, MessageRecoveryState,
    };
    for action in [MessageRecoveryAction::Observe, MessageRecoveryAction::Adopt] {
        let params = serde_json::value::to_raw_value(&serde_json::json!({
            "sessionId": parent.0, "action": action, "recoveryRef": reference,
        }))
        .unwrap();
        let response = bounded(acp::Agent::ext_method(
            &agent,
            acp::ExtRequest::new("x.ai/subagent/message/recover", params.into()),
        ))
        .await
        .unwrap();
        let wire: serde_json::Value = serde_json::from_str(response.0.get()).unwrap();
        assert_eq!(
            wire["result"],
            serde_json::to_value(
                crate::extensions::subagent_message::SendSubagentMessageOutcome::RecoveryObserved {
                    recovery_ref: reference.clone(),
                    state: MessageRecoveryState::Succeeded,
                },
            )
            .unwrap()
        );
    }
    let other_parent = agent
        .new_session_inner(
            acp::NewSessionRequest::new(repository.clone()).meta(
                serde_json::json!({ "modelId": "test-model" })
                    .as_object()
                    .cloned(),
            ),
        )
        .await
        .unwrap()
        .session_id;
    let params = serde_json::value::to_raw_value(&serde_json::json!({
        "sessionId": other_parent.0, "action": MessageRecoveryAction::Adopt,
        "recoveryRef": reference,
    }))
    .unwrap();
    let response = bounded(acp::Agent::ext_method(
        &agent,
        acp::ExtRequest::new("x.ai/subagent/message/recover", params.into()),
    ))
    .await
    .unwrap();
    let wire: serde_json::Value = serde_json::from_str(response.0.get()).unwrap();
    assert_eq!(wire["result"]["kind"], "rejected");
    assert_eq!(
        server.request_count(),
        sampled,
        "observe/adopt never resend a completed original input"
    );
    let conversation = reopened.handle.chat_state_handle.get_conversation().await;
    assert_eq!(
        serde_json::to_string(&conversation)
            .unwrap()
            .matches(WAKE_TEXT)
            .count(),
        1
    );
}

#[test]
fn completed_native_worker_receiver_contention_is_unconfirmed_without_model_input() {
    xai_test_utils::require_git!();
    run_local_for_bridge_test(|| async {
        let server = xai_grok_test_support::MockInferenceServer::start()
            .await
            .unwrap();
        server.set_response("preserved native result");
        server.set_keep_requests(true);
        let agent = build_minimal_agent_for_tests();
        let mut model =
            crate::agent::config::ModelEntry::fallback("test-model", &Default::default());
        model.info.base_url = server.url();
        model.info.api_backend = crate::sampling::ApiBackend::Responses;
        model.api_key = Some("test".into());
        agent.models_manager.insert_test_entry("test-model", model);
        let directory = tempfile::tempdir().unwrap();
        let (repository, _remote) = xai_test_utils::git::seed_repo_with_remote(directory.path());
        agent.cfg.borrow_mut().feature_values.insert(
            crate::agent::config::Feature::SubagentWorktreeSnapshot,
            true,
        );
        let parent = super::super::new_root_session(&agent, &repository).await;
        let parent_info = agent.resident_handle(&parent).unwrap().info.clone();
        agent.start_subagent_coordinator();
        let id = uuid::Uuid::now_v7().to_string();
        let backend = ChannelBackend::for_coordinator_session(
            agent.subagent_event_tx.clone(),
            parent.0.to_string(),
        );
        let mut request = auto_wake_test_request(&id);
        request.parent_session_id = parent.0.to_string();
        request.prompt = "Complete the task before the receiver contention.".into();
        request.run_in_background = false;
        request.await_to_completion = true;
        request.runtime_overrides.isolation = Some(xai_tool_types::SubagentIsolationMode::Worktree);
        let prior = bounded(backend.spawn(request, None, None)).await.unwrap();
        assert!(prior.success(), "{prior:?}");
        let persisted =
            crate::agent::subagent::durable_resume_source(&id, parent.0.as_ref(), &repository)
                .unwrap();
        let snapshot_ref = persisted
            .snapshot_ref
            .clone()
            .expect("completed native task retains a snapshot");
        let resolve_snapshot = || {
            let result = std::process::Command::new("git")
                .current_dir(&repository)
                .args(["rev-parse", snapshot_ref.as_str()])
                .output()
                .unwrap();
            assert!(result.status.success());
            result.stdout
        };
        let original_snapshot = resolve_snapshot();
        let info = crate::session::info::Info {
            id: acp::SessionId::new(id.clone()),
            cwd: persisted.child_cwd,
        };
        let directory = crate::session::persistence::session_dir(&info);
        let artifacts = crate::session::persistence::session_dir(&parent_info)
            .join("subagents")
            .join(&id);
        let prior_meta = std::fs::read(artifacts.join("meta.json")).unwrap();
        let prior_output = std::fs::read(artifacts.join("output.json")).unwrap();
        let prior_history = std::fs::read(directory.join("chat_history.jsonl")).unwrap();
        let storage = JsonlStorageAdapter::with_root(crate::util::grok_home::grok_home());
        let (lease, recovered) = storage.claim_receiver(&info).await.unwrap();
        let requests_before = server.request_count();
        let outcome = bounded(
            backend
                .send_active_message(ActiveAgentMessageRequest::try_new(&id, WAKE_TEXT).unwrap()),
        )
        .await;
        let ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref } = outcome else {
            panic!("native admission must remain unconfirmed: {outcome:?}");
        };
        assert_eq!(recovery_ref.receiver.session_id, id);
        assert!(uuid::Uuid::parse_str(&recovery_ref.message_id).is_ok());
        assert_eq!(recovery_ref.operation,
            xai_grok_tools::implementations::grok_build::task::types::ActiveAgentMessageOperation::Queue);
        assert_eq!(
            recovery_ref.prompt_id,
            format!("parent-message-{}", recovery_ref.message_id)
        );
        assert_eq!(
            recovery_ref.delivery_id,
            format!("parent-message/{}", recovery_ref.message_id)
        );
        use sha2::{Digest, Sha256};
        assert_eq!(
            recovery_ref.payload_digest,
            format!("{:x}", Sha256::digest(WAKE_TEXT.as_bytes()))
        );
        let restored = bounded(backend.query(&id, true, Some(30_000), None))
            .await
            .unwrap();
        assert!(
            matches!(restored.status, SubagentSnapshotStatus::Completed { ref output, .. }
            if output.as_str() == prior.output.as_ref())
        );
        assert_eq!(server.request_count(), requests_before);
        assert_eq!(resolve_snapshot(), original_snapshot);
        assert_eq!(
            std::fs::read(artifacts.join("meta.json")).unwrap(),
            prior_meta
        );
        assert_eq!(
            std::fs::read(artifacts.join("output.json")).unwrap(),
            prior_output
        );
        assert_eq!(
            std::fs::read(directory.join("chat_history.jsonl")).unwrap(),
            prior_history
        );
        let failed = storage.load_summary(&info).await.unwrap();
        assert_eq!(failed.attempt_id, recovered.attempt_id);
        assert_eq!(failed.next_trace_turn, recovered.next_trace_turn);
        assert_eq!(
            failed.completion_delivery.receiver.records.len(),
            recovered.completion_delivery.receiver.records.len()
        );
        drop(lease);
        let reopened = bounded(agent.management_session(&info.id)).await.unwrap();
        let conversation = reopened.handle.chat_state_handle.get_conversation().await;
        assert!(
            !serde_json::to_string(&conversation)
                .unwrap()
                .contains(WAKE_TEXT)
        );
        assert_eq!(
            server.request_count(),
            requests_before,
            "unconfirmed admission is not a fabricated model input"
        );
    });
}
