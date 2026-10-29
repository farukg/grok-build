use super::*;
use crate::agent::subagent::completion_test_support::auto_wake_test_request;
use crate::session::delivery_receiver::{
    DeliveryExecution, DeliveryProvenance, ParentMessageCustody,
};
use crate::session::persistence::native_wake_tests_support::ParentInputFault;
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter};
use acp::Agent as _;
use xai_grok_test_support::{
    InferenceEndpoint, InferenceRequestMatcher, MockInferenceServer, ScriptedResponse,
};
use xai_grok_tools::implementations::grok_build::task::backend::{
    ChannelBackend, SubagentBackend, SubagentCoordinatorSender,
};
use xai_grok_tools::implementations::grok_build::task::coordinator::{
    CoordinatorConfig, SubagentCoordinator,
};
use xai_grok_tools::implementations::grok_build::task::native_recovery::{
    MessageRecoveryAction, MessageRecoveryRef, MessageRecoveryState,
};
use xai_grok_tools::implementations::grok_build::task::types::{
    ActiveAgentMessageOutcome, ActiveAgentMessageRequest, ActiveMessageSenderContext,
    SubagentSnapshotStatus,
};

#[path = "native_adoption_attempt_teardown.rs"]
mod adoption_attempt_teardown;
#[path = "native_adoption_explicit_cancel.rs"]
mod adoption_explicit_cancel;

const TEXT: &str = "recover-exact-native-admission-921e";

fn original_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false})
}

enum TestArtifacts {
    Disabled,
    UnnamedDirect,
    AuthenticatedProxy,
}

#[track_caller]
fn bounded<T>(
    future: impl std::future::Future<Output = T>,
) -> impl std::future::Future<Output = T> {
    let caller = std::panic::Location::caller();
    async move {
        tokio::time::timeout(std::time::Duration::from_secs(45), future)
            .await
            .unwrap_or_else(|_| panic!("native adoption boundary timed out at {caller}"))
    }
}

fn coordinator(
    agent: &MvpAgent,
    fault: Option<ParentInputFault>,
    artifacts: Option<(String, crate::session::repo_changes::UploadMethod)>,
) -> (
    SubagentCoordinatorSender,
    tokio_util::task::AbortOnDropHandle<()>,
) {
    let (sender, receiver) = SubagentCoordinator::<ShellChildRunner>::channel();
    let runner = ShellChildRunner {
        agent_ref: LocalRef::new(agent),
        presentations: Default::default(),
        native_wake_fault: std::cell::RefCell::new(fault),
        native_wake_artifacts: artifacts,
        native_coordinator_tx: Some(sender.event_sender().0),
    };
    let task = tokio::task::spawn_local(
        SubagentCoordinator::from_channel(receiver, runner, CoordinatorConfig::default()).run(),
    );
    (sender, tokio_util::task::AbortOnDropHandle::new(task))
}

struct Fixture {
    agent: Box<MvpAgent>,
    server: MockInferenceServer,
    parent: acp::SessionId,
    info: crate::session::info::Info,
    storage: JsonlStorageAdapter,
    directory: std::path::PathBuf,
    artifacts: std::path::PathBuf,
    prior_meta: Vec<u8>,
    prior_output: Vec<u8>,
    prior_history: Vec<u8>,
    prior_recipe: Vec<u8>,
    prior_attempt: Option<String>,
    prior_next_turn: u64,
    prior_result: xai_grok_tools::implementations::grok_build::task::types::SubagentResult,
    reference: MessageRecoveryRef,
    artifact_binding: Option<(String, crate::session::repo_changes::UploadMethod)>,
    _repo: tempfile::TempDir,
    _auth: tempfile::TempDir,
    gateway: mpsc::UnboundedReceiver<xai_acp_lib::AcpClientMessage>,
}

async fn lost_owner(fault: ParentInputFault) -> Fixture {
    lost_owner_with_artifacts(fault, TestArtifacts::Disabled).await
}

async fn lost_owner_with_artifacts(fault: ParentInputFault, artifacts: TestArtifacts) -> Fixture {
    lost_owner_from(
        fault,
        artifacts,
        crate::agent::subagent::ActiveAgentMessageSource::Agent,
    )
    .await
}

async fn lost_owner_from(
    fault: ParentInputFault,
    artifacts: TestArtifacts,
    source: crate::agent::subagent::ActiveAgentMessageSource,
) -> Fixture {
    let server = MockInferenceServer::start().await.unwrap();
    server.set_keep_requests(true);
    let auth_directory = tempfile::tempdir().unwrap();
    let mut config = crate::agent::config::Config::default();
    config.telemetry.trace_upload = Some(false);
    config
        .feature_values
        .insert(crate::agent::config::Feature::ActiveAgentMessages, true);
    config.endpoints.cli_chat_proxy_base_url = Some(server.url());
    config.endpoints.xai_api_base_url = server.url();
    config.endpoints.models_base_url = Some(server.url());
    config.endpoints.models_list_url = Some(format!("{}/models", server.url()));
    config.endpoints.trace_upload_url = Some(server.url());
    config.endpoints.feedback_base_url = Some(server.url());
    config.endpoints.managed_config_url = Some(format!("{}/deployment/config", server.url()));
    config.grok_com_config.grok_ws_origin = server.url();
    config.grok_com_config.grok_ws_url = format!("{}/ws", server.url().replacen("http", "ws", 1));
    config.grok_com_config.oidc = Some(xai_grok_login::OidcAuthConfig {
        issuer: server.origin(),
        client_id: "native-adoption-fixture".into(),
        scopes: Vec::new(),
        audience: None,
    });
    config.grok_com_config.oauth2 = None;
    let auth_manager = std::sync::Arc::new(crate::auth::AuthManager::new(
        auth_directory.path(),
        config.grok_com_config.clone(),
    ));
    let (gateway_tx, mut gateway) = mpsc::unbounded_channel();
    let agent = Box::new(
        MvpAgent::new(
            GatewaySender::new(gateway_tx),
            &config,
            auth_manager,
            None,
            None,
        )
        .unwrap(),
    );
    agent
        .auth_manager
        .hot_swap(xai_grok_login::GrokAuth::test_default());
    let artifacts = match artifacts {
        TestArtifacts::Disabled => None,
        TestArtifacts::UnnamedDirect => Some((
            "gs://native-recovery-artifacts".into(),
            crate::session::repo_changes::UploadMethod::Direct {
                service_account_key: None,
            },
        )),
        TestArtifacts::AuthenticatedProxy => Some((
            "gs://native-recovery-artifacts".into(),
            crate::session::repo_changes::UploadMethod::Proxy {
                proxy_base_url: server.url(),
                user_token: String::new(),
                deployment_key: None,
                alpha_test_key: None,
            },
        )),
    };
    let artifact_binding = artifacts.clone();
    let mut model = crate::agent::config::ModelEntry::fallback("test-model", &Default::default());
    model.info.base_url = server.url();
    model.info.api_backend = crate::sampling::ApiBackend::Responses;
    model.api_key = Some("native-fixture-only".into());
    agent.models_manager.insert_test_entry("test-model", model);
    agent
        .auth_method_id
        .store(Some(std::sync::Arc::new(acp::AuthMethodId::new(
            "cached_token",
        ))));
    bounded(
        agent.initialize(
            acp::InitializeRequest::new(acp::ProtocolVersion::V1).client_capabilities(
                acp::ClientCapabilities::new()
                    .fs(acp::FileSystemCapabilities::new())
                    .terminal(false),
            ),
        ),
    )
    .await
    .unwrap();
    let repo = tempfile::tempdir().unwrap();
    let (repository, _remote) = xai_test_utils::git::seed_repo_with_remote(repo.path());
    let parent = bounded(agent.new_session(acp::NewSessionRequest::new(repository.clone()).meta(
        serde_json::json!({ "sessionId": uuid::Uuid::now_v7().to_string(), "modelId": "test-model" }).as_object().cloned(),
    ))).await.unwrap().session_id;
    let (sender, mut owner) = coordinator(&agent, Some(fault.clone()), artifacts);
    let backend = ChannelBackend::for_coordinator_session(sender, parent.0.to_string());
    let id = uuid::Uuid::now_v7().to_string();
    let mut request = auto_wake_test_request(&id);
    request.parent_session_id = parent.0.to_string();
    request.prompt = "complete-before-native-recovery".into();
    request.run_in_background = false;
    request.await_to_completion = true;
    request.runtime_overrides.output_schema = Some(original_schema());
    let original = server.expect_response(
        "original native result",
        InferenceRequestMatcher::foreground_containing(
            InferenceEndpoint::Responses,
            "complete-before-native-recovery",
        ),
        ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact(
            r#"{"value":"old-completed-native-result"}"#,
            "test-model",
        )),
    );
    let prior = bounded(backend.spawn(request, None, None)).await.unwrap();
    assert!(prior.success(), "{prior:?}");
    original.assert_satisfied();
    let persisted_source =
        crate::agent::subagent::durable_resume_source(&id, parent.0.as_ref(), &repository).unwrap();
    let info = crate::session::info::Info {
        id: acp::SessionId::new(id.clone()),
        cwd: persisted_source.child_cwd,
    };
    let directory = crate::session::persistence::session_dir(&info);
    let parent_info = agent.resident_handle(&parent).unwrap().info;
    let artifacts = crate::session::persistence::session_dir(&parent_info)
        .join("subagents")
        .join(&id);
    let prior_meta = std::fs::read(artifacts.join("meta.json")).unwrap();
    let prior_output = std::fs::read(artifacts.join("output.json")).unwrap();
    let prior_history = std::fs::read(directory.join("chat_history.jsonl")).unwrap();
    let prior_recipe = std::fs::read(directory.join("child_management.json")).unwrap();
    let storage = JsonlStorageAdapter::with_root(crate::util::grok_home::grok_home());
    let before = storage.load_summary(&info).await.unwrap();
    let sampled = server.request_count();
    agent
        .try_build_subagent_spawn_context(parent.0.as_ref())
        .unwrap()
        .active_message_parent_prompt_index
        .store(1729, std::sync::atomic::Ordering::Release);
    let message = match source {
        crate::agent::subagent::ActiveAgentMessageSource::Agent => {
            ActiveAgentMessageRequest::try_new(&id, TEXT).unwrap()
        }
        crate::agent::subagent::ActiveAgentMessageSource::Human => {
            use crate::agent::mvp_agent::{ChildHost, ChildReach, SessionHost};
            let Some(SessionHost::Child(ChildHost {
                reach: ChildReach::Addressed { address, .. },
                ..
            })) = agent.session_host(&info.id)
            else {
                panic!("original human route");
            };
            ActiveAgentMessageRequest::try_new_from_human(address.as_str(), TEXT,
                xai_grok_tools::implementations::grok_build::task::types::ActiveAgentMessageOperation::Queue).unwrap()
        }
    };
    while gateway.try_recv().is_ok() {}
    let outcome = bounded(backend.send_active_message(message)).await;
    let reference = match fault {
        ParentInputFault::BeforeAdoptedProjection { .. }
        | ParentInputFault::ReleasedAttemptEnd { .. }
        | ParentInputFault::PublishedReconfirm { .. } => {
            panic!("initial-owner fixture requires an initial-owner cut")
        }
        ParentInputFault::ShutdownBeforeProjection => {
            let ActiveAgentMessageOutcome::Accepted { message_id } = outcome else {
                panic!("published cut must follow real acceptance: {outcome:?}");
            };
            bounded(async {
                loop {
                    let message = gateway.recv().await.unwrap();
                    if matches!(message, xai_acp_lib::AcpClientMessage::ExtNotification(args) if args.request.params.get().contains("subagent_spawned")) { break; }
                }
            }).await;
            let live = bounded(agent.management_session(&info.id)).await.unwrap();
            let handle = live.handle.clone();
            drop(live);
            owner.abort();
            assert!(bounded(&mut owner).await.unwrap_err().is_cancelled());
            agent.request_session_shutdown(&info.id);
            bounded(handle.cmd_tx.closed()).await;
            agent.drain_old_session_thread(&info.id).await;
            let summary = storage.load_summary(&info).await.unwrap();
            summary.completion_delivery.receiver.records[&format!("parent-message/{message_id}")]
                .parent_recovery_ref()
                .unwrap()
        }
        ParentInputFault::AtomicWrite(_)
        | ParentInputFault::LoseAdmissionAck
        | ParentInputFault::LoseReservationAck
        | ParentInputFault::ReservationDigestConflict
        | ParentInputFault::ShutdownAfterAdmission
        | ParentInputFault::ShutdownAfterProjection
        | ParentInputFault::ShutdownAfterWakeStart => {
            let ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref } = outcome else {
                panic!("lost owner must retain recovery identity: {outcome:?}");
            };
            recovery_ref
        }
    };
    assert_eq!(reference.receiver.session_id, id);
    assert_eq!(
        reference.prompt_id,
        format!("parent-message-{}", reference.message_id)
    );
    assert_eq!(
        reference.delivery_id,
        format!("parent-message/{}", reference.message_id)
    );
    use sha2::{Digest, Sha256};
    assert_eq!(
        reference.payload_digest,
        format!("{:x}", Sha256::digest(TEXT.as_bytes()))
    );
    match fault {
        ParentInputFault::BeforeAdoptedProjection { .. }
        | ParentInputFault::ReleasedAttemptEnd { .. }
        | ParentInputFault::PublishedReconfirm { .. } => {
            panic!("initial-owner fixture requires an initial-owner cut")
        }
        ParentInputFault::ShutdownBeforeProjection => {
            let meta: crate::agent::subagent::SubagentMeta =
                serde_json::from_slice(&std::fs::read(artifacts.join("meta.json")).unwrap())
                    .unwrap();
            assert_eq!(
                meta.status,
                crate::agent::subagent::SubagentMetaStatus::Running
            );
            assert_ne!(meta.attempt_id, before.attempt_id);
        }
        ParentInputFault::AtomicWrite(_)
        | ParentInputFault::LoseAdmissionAck
        | ParentInputFault::LoseReservationAck
        | ParentInputFault::ReservationDigestConflict
        | ParentInputFault::ShutdownAfterAdmission
        | ParentInputFault::ShutdownAfterProjection
        | ParentInputFault::ShutdownAfterWakeStart => {
            let restored = bounded(backend.query(&id, true, Some(30_000), None))
                .await
                .unwrap();
            assert!(matches!(
                restored.status,
                SubagentSnapshotStatus::Completed { .. }
            ));
            assert_eq!(
                std::fs::read(artifacts.join("meta.json")).unwrap(),
                prior_meta
            );
        }
    }
    assert_eq!(
        sampled,
        server.request_count(),
        "native owner loss must not dispatch; routes={:?}",
        server
            .requests()
            .into_iter()
            .map(|entry| (entry.method, entry.path, entry.endpoint, entry.conversation))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        std::fs::read(artifacts.join("output.json")).unwrap(),
        prior_output
    );
    assert_eq!(
        std::fs::read(directory.join("chat_history.jsonl")).unwrap(),
        prior_history
    );
    assert_eq!(
        std::fs::read(directory.join("child_management.json")).unwrap(),
        prior_recipe
    );
    let durable = storage.load_summary(&info).await.unwrap();
    let record = &durable.completion_delivery.receiver.records[&reference.delivery_id];
    assert!(matches!(record.execution, DeliveryExecution::Queued { .. }));
    match fault {
        ParentInputFault::BeforeAdoptedProjection { .. }
        | ParentInputFault::ReleasedAttemptEnd { .. }
        | ParentInputFault::PublishedReconfirm { .. } => {
            panic!("initial-owner fixture requires an initial-owner cut")
        }
        ParentInputFault::ShutdownAfterWakeStart | ParentInputFault::ShutdownBeforeProjection => {
            let DeliveryProvenance::ParentMessage { admission } = &record.provenance else {
                panic!("native admission");
            };
            let ParentMessageCustody::Initial { custody, .. } = &admission.custody else {
                panic!("native custody");
            };
            assert_eq!(
                durable.attempt_id.as_deref(),
                Some(custody.launch.attempt_id.as_str())
            );
            assert_eq!(durable.next_trace_turn, before.next_trace_turn + 1);
        }
        ParentInputFault::AtomicWrite(_)
        | ParentInputFault::LoseAdmissionAck
        | ParentInputFault::LoseReservationAck
        | ParentInputFault::ReservationDigestConflict
        | ParentInputFault::ShutdownAfterAdmission
        | ParentInputFault::ShutdownAfterProjection => {
            assert_eq!(before.attempt_id, durable.attempt_id);
            assert_eq!(before.next_trace_turn, durable.next_trace_turn);
        }
    }
    drop(backend);
    drop(owner);
    Fixture {
        agent,
        server,
        parent,
        info,
        storage,
        directory,
        artifacts,
        prior_meta,
        prior_output,
        prior_history,
        prior_recipe,
        prior_attempt: before.attempt_id,
        prior_next_turn: before.next_trace_turn,
        prior_result: prior,
        reference,
        artifact_binding,
        _repo: repo,
        _auth: auth_directory,
        gateway,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn native_lost_owner_adopts_durable_initial_input_once_with_original_schema_and_results() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner_with_artifacts(ParentInputFault::ShutdownAfterAdmission, TestArtifacts::AuthenticatedProxy).await;
        let original = f.storage.load_summary(&f.info).await.unwrap();
        let DeliveryProvenance::ParentMessage { admission } = &original.completion_delivery.receiver.records[&f.reference.delivery_id].provenance else { panic!("parent custody"); };
        let ParentMessageCustody::Initial { custody, .. } = &admission.custody else { panic!("native initial custody"); };
        let saved_custody = serde_json::to_value(&admission.custody).unwrap();
        let saved = custody.clone();
        assert_eq!(std::fs::read(f.directory.join("child_management.json")).unwrap(), f.prior_recipe);
        assert_eq!(saved.launch.runtime_overrides.output_schema, Some(original_schema()));
        assert_eq!(saved.launch.traceparent.as_deref(), Some("00-0123456789abcdef0123456789abcdef-0123456789abcdef-01"));
        assert_eq!(saved.launch.parent_prompt_index, 1729);
        assert_eq!(Some(saved.launch.previous_completion.attempt_id.clone()), f.prior_attempt);
        let previous = serde_json::to_value(&saved.launch.previous_completion).unwrap();
        assert_eq!(previous["result"]["tokens_used"], f.prior_result.tokens_used);
        assert_eq!(previous["result"]["output_tokens_used"], f.prior_result.output_tokens_used);
        assert_eq!(previous["result"]["total_tokens_used"], f.prior_result.total_tokens_used);
        assert_eq!(previous["result"]["output_usage_incomplete"], f.prior_result.output_usage_incomplete);
        let wire = serde_json::to_string(&saved).unwrap();
        assert!(!wire.contains(TEXT));
        assert!(!wire.contains("old-completed-native-result"));
        assert!(!wire.contains("complete-before-native-recovery"));
        for forbidden in ["user_token", "service_account_key", "credentials_content", "deployment_key", "alpha_test_key"] {
            assert!(!wire.contains(forbidden));
        }
        f.agent.try_build_subagent_spawn_context(f.parent.0.as_ref()).unwrap().active_message_parent_prompt_index.store(9999, std::sync::atomic::Ordering::Release);
        let (sender, owner) = coordinator(&f.agent, None, f.artifact_binding.clone());
        let backend = ChannelBackend::for_coordinator_session(sender.clone(), f.parent.0.to_string());
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Observe)).await,
            ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref: f.reference.clone(), state: MessageRecoveryState::Queued });
        assert_eq!(std::fs::read(f.artifacts.join("meta.json")).unwrap(), f.prior_meta);
        assert_eq!(std::fs::read(f.artifacts.join("output.json")).unwrap(), f.prior_output);
        assert_eq!(std::fs::read(f.directory.join("chat_history.jsonl")).unwrap(), f.prior_history);
        let sampled = f.server.request_count();
        let mut resumed = f.server.expect_response_blocked("adopted native result", InferenceRequestMatcher::foreground_containing(
            InferenceEndpoint::Responses, TEXT), ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact(r#"{"value":"adopted-native-result"}"#, "test-model")));
        let (first, second) = bounded(async {
            tokio::join!(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt),
                backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt))
        }).await;
        let outcomes = [first, second];
        assert_eq!(outcomes.iter().filter(|outcome| matches!(outcome, ActiveAgentMessageOutcome::Accepted { message_id } if message_id == &f.reference.message_id)).count(), 1);
        assert!(outcomes.iter().all(|outcome| matches!(outcome,
            ActiveAgentMessageOutcome::Accepted { message_id } if message_id == &f.reference.message_id)
            || matches!(outcome, ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref } if recovery_ref == &f.reference)
            || matches!(outcome, ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref, state: MessageRecoveryState::Processing } if recovery_ref == &f.reference)));
        bounded(resumed.wait_blocked()).await;
        let requests = f.server.request_bodies();
        let adopted = requests.iter().filter(|request| request.to_string().contains(TEXT)).collect::<Vec<_>>();
        assert_eq!(adopted.len(), 1);
        assert_eq!(adopted[0].to_string().matches(TEXT).count(), 1);
        assert_eq!(adopted[0].pointer("/text/format/schema"), Some(&original_schema()));
        let observation = bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await;
        assert!(matches!(observation, ActiveAgentMessageOutcome::RecoveryObserved { state: MessageRecoveryState::Processing, .. }));
        assert_eq!(f.server.request_count(), sampled + 1);
        resumed.release();
        let done = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.unwrap();
        assert!(matches!(done.status, SubagentSnapshotStatus::Completed { output, .. } if output.contains("adopted-native-result")));
        resumed.assert_satisfied();
        let completed = f.storage.load_summary(&f.info).await.unwrap();
        let record = &completed.completion_delivery.receiver.records[&f.reference.delivery_id];
        assert!(matches!(record.execution, DeliveryExecution::Succeeded));
        assert_eq!(completed.attempt_id.as_deref(), Some(saved.launch.attempt_id.as_str()));
        assert_ne!(completed.attempt_id, f.prior_attempt);
        assert_eq!(completed.next_trace_turn, f.prior_next_turn + 1);
        let DeliveryProvenance::ParentMessage { admission } = &record.provenance else { panic!("parent provenance"); };
        assert_eq!(admission.sender_session_id, f.parent.0.as_ref());
        assert_eq!(serde_json::to_value(&admission.custody).unwrap(), saved_custody);
        let history = std::fs::read_to_string(f.directory.join("chat_history.jsonl")).unwrap();
        let bindings = history.lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap().get("_session_delivery").cloned())
            .filter(|binding| binding["delivery_id"] == f.reference.delivery_id).collect::<Vec<_>>();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0]["turn_id"], f.reference.prompt_id);
        assert_eq!(bindings[0]["payload_digest"], f.reference.payload_digest);
        let binding: xai_chat_state::DeliveryBinding = serde_json::from_value(bindings[0].clone()).unwrap();
        assert!(matches!(binding.parent_task, xai_chat_state::ParentTask::Instance { id } if !id.is_empty()));
        let loaded = f.storage.load_session(&f.info).await.unwrap();
        assert!(loaded.chat_history.iter().any(|item| matches!(item,
            xai_grok_sampling_types::conversation::ConversationItem::User(user)
                if user.synthetic_reason == xai_grok_sampling_types::conversation::SyntheticReason::AgentMessage
                    && serde_json::to_string(user).unwrap().contains(TEXT))));
        assert!(f.server.storage_request_count() > 0, "original proxy artifact capability must actually be used");
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await,
            ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref: f.reference, state: MessageRecoveryState::Succeeded });
        assert_eq!(f.server.request_count(), sampled + 1);
        drop(owner);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_lost_ack_recovery_refuses_wrong_audience_generation_and_digest_without_touching_old_completion()
 {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner(ParentInputFault::LoseAdmissionAck).await;
        let (sender, owner) = coordinator(&f.agent, None, None);
        let sampled = f.server.request_count();
        let mut wrong_digest = f.reference.clone();
        wrong_digest.payload_digest.push('0');
        let mut wrong_prompt = f.reference.clone();
        wrong_prompt.prompt_id.push('0');
        for reference in [wrong_digest, wrong_prompt] {
            assert_eq!(bounded(sender.recover_message(reference, MessageRecoveryAction::Adopt,
                ActiveMessageSenderContext::RootSession { session_id: std::sync::Arc::from(f.parent.0.as_ref()) })).await,
                ActiveAgentMessageOutcome::NotFoundOrNotOwned);
        }
        let mut generation = f.reference.clone();
        generation.receiver.generation = xai_grok_tools::implementations::grok_build::task::root_control::AgentMessageGeneration::mint(912);
        assert_eq!(bounded(sender.recover_message(generation, MessageRecoveryAction::Adopt,
            ActiveMessageSenderContext::RootSession { session_id: std::sync::Arc::from(f.parent.0.as_ref()) })).await,
            ActiveAgentMessageOutcome::NotFoundOrNotOwned);
        assert_eq!(bounded(sender.recover_message(f.reference.clone(), MessageRecoveryAction::Observe,
            ActiveMessageSenderContext::HumanRoot { session_id: std::sync::Arc::from(f.parent.0.as_ref()) })).await,
            ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref: f.reference.clone(), state: MessageRecoveryState::Queued });
        assert_eq!(bounded(sender.recover_message(f.reference, MessageRecoveryAction::Adopt,
            ActiveMessageSenderContext::RootSession { session_id: std::sync::Arc::from("unrelated-audience") })).await,
            ActiveAgentMessageOutcome::NotFoundOrNotOwned);
        assert_eq!(sampled, f.server.request_count());
        assert_eq!(std::fs::read(f.artifacts.join("meta.json")).unwrap(), f.prior_meta);
        assert_eq!(std::fs::read(f.artifacts.join("output.json")).unwrap(), f.prior_output);
        assert_eq!(std::fs::read(f.directory.join("chat_history.jsonl")).unwrap(), f.prior_history);
        drop(owner);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_unavailable_original_artifact_binding_is_correlated_uncertainty_not_default_substitution()
 {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner_with_artifacts(ParentInputFault::ShutdownAfterAdmission, TestArtifacts::UnnamedDirect).await;
        let sampled = f.server.request_count();
        let original = f.storage.load_summary(&f.info).await.unwrap();
        let (sender, owner) = coordinator(&f.agent, None, None);
        let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await,
            ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref: f.reference.clone() });
        let preserved = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.unwrap();
        assert!(matches!(&preserved.status, SubagentSnapshotStatus::Completed { output, tool_calls, turns, worktree_path }
            if output.as_str() == f.prior_result.output.as_ref() && *tool_calls == f.prior_result.tool_calls
                && *turns == f.prior_result.turns && worktree_path == &f.prior_result.worktree_path));
        let observed = bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Observe)).await;
        assert_eq!(observed, ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref: f.reference.clone(), state: MessageRecoveryState::Queued });
        assert_eq!(f.server.request_count(), sampled);
        assert_eq!(std::fs::read(f.artifacts.join("meta.json")).unwrap(), f.prior_meta);
        assert_eq!(std::fs::read(f.artifacts.join("output.json")).unwrap(), f.prior_output);
        assert_eq!(std::fs::read(f.directory.join("chat_history.jsonl")).unwrap(), f.prior_history);
        let unchanged = f.storage.load_summary(&f.info).await.unwrap();
        assert_eq!(serde_json::to_value(&unchanged.completion_delivery.receiver.records[&f.reference.delivery_id]).unwrap(),
            serde_json::to_value(&original.completion_delivery.receiver.records[&f.reference.delivery_id]).unwrap());
        assert_eq!(unchanged.attempt_id, f.prior_attempt);
        assert_eq!(unchanged.next_trace_turn, f.prior_next_turn);
        let overtaking = bounded(backend.send_active_message(ActiveAgentMessageRequest::try_new(
            &f.reference.receiver.session_id, "new message cannot replace the admitted native intent").unwrap())).await;
        assert!(matches!(overtaking, ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref }
            if recovery_ref.receiver == f.reference.receiver && recovery_ref.message_id != f.reference.message_id));
        let protected = f.storage.load_summary(&f.info).await.unwrap();
        assert_eq!(protected.completion_delivery.receiver.records.len(), original.completion_delivery.receiver.records.len());
        assert_eq!(serde_json::to_value(&protected.completion_delivery.receiver.records[&f.reference.delivery_id]).unwrap(),
            serde_json::to_value(&original.completion_delivery.receiver.records[&f.reference.delivery_id]).unwrap());
        assert_eq!(f.server.request_count(), sampled);
        assert_eq!(std::fs::read(f.artifacts.join("meta.json")).unwrap(), f.prior_meta);
        assert_eq!(std::fs::read(f.artifacts.join("output.json")).unwrap(), f.prior_output);
        drop(owner);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_projected_owner_loss_is_observed_interrupted_and_never_replayed() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new()
        .run_until(async {
            let f = lost_owner(ParentInputFault::ShutdownAfterAdmission).await;
            let sampled = f.server.request_count();
            let (sender, owner) = coordinator(
                &f.agent,
                Some(ParentInputFault::ShutdownAfterProjection),
                None,
            );
            let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
            assert_eq!(
                bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt))
                    .await,
                ActiveAgentMessageOutcome::Accepted {
                    message_id: f.reference.message_id.clone()
                }
            );
            let done =
                bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None))
                    .await
                    .unwrap();
            assert!(!matches!(
                done.status,
                SubagentSnapshotStatus::Running { .. }
            ));
            assert_eq!(
                sampled,
                f.server.request_count(),
                "lost strict projection ack never reaches inference"
            );
            drop(backend);
            drop(owner);
            let (lease, interrupted) = f.storage.claim_receiver(&f.info).await.unwrap();
            assert!(matches!(
                interrupted.completion_delivery.receiver.records[&f.reference.delivery_id]
                    .execution,
                DeliveryExecution::Interrupted { .. }
            ));
            drop(lease);
            let history = std::fs::read(f.directory.join("chat_history.jsonl")).unwrap();
            let (sender, owner) = coordinator(&f.agent, None, None);
            let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
            for action in [MessageRecoveryAction::Observe, MessageRecoveryAction::Adopt] {
                assert_eq!(
                    bounded(backend.recover_message(f.reference.clone(), action)).await,
                    ActiveAgentMessageOutcome::RecoveryObserved {
                        recovery_ref: f.reference.clone(),
                        state: MessageRecoveryState::Interrupted
                    }
                );
            }
            assert_eq!(
                history,
                std::fs::read(f.directory.join("chat_history.jsonl")).unwrap()
            );
            assert_eq!(sampled, f.server.request_count());
            drop(owner);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_startup_observes_same_queued_identity_without_inference_or_completed_artifact_changes()
 {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut f = lost_owner(ParentInputFault::ShutdownAfterAdmission).await;
            while f.gateway.try_recv().is_ok() {}
            let sampled = f.server.request_count();
            let reopened = bounded(f.agent.management_session(&f.info.id))
                .await
                .unwrap();
            let conversation = reopened.handle.chat_state_handle.get_conversation().await;
            assert!(!serde_json::to_string(&conversation).unwrap().contains(TEXT));
            let expected = serde_json::to_value(&f.reference).unwrap();
            bounded(async {
                loop {
                    let message = f.gateway.recv().await.unwrap();
                    if let xai_acp_lib::AcpClientMessage::ExtNotification(notification) = message {
                        let payload: serde_json::Value =
                            serde_json::from_str(notification.request.params.get()).unwrap();
                        if payload.get("recoveryRef") == Some(&expected) {
                            assert_eq!(payload["sessionId"], f.reference.receiver.session_id);
                            break;
                        }
                    }
                }
            })
            .await;
            assert_eq!(sampled, f.server.request_count());
            assert_eq!(
                std::fs::read(f.artifacts.join("meta.json")).unwrap(),
                f.prior_meta
            );
            assert_eq!(
                std::fs::read(f.artifacts.join("output.json")).unwrap(),
                f.prior_output
            );
            assert_eq!(
                std::fs::read(f.directory.join("chat_history.jsonl")).unwrap(),
                f.prior_history
            );
            let unchanged = f.storage.load_summary(&f.info).await.unwrap();
            assert!(matches!(
                unchanged.completion_delivery.receiver.records[&f.reference.delivery_id].execution,
                DeliveryExecution::Queued { .. }
            ));
            assert_eq!(unchanged.attempt_id, f.prior_attempt);
            assert_eq!(unchanged.next_trace_turn, f.prior_next_turn);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_human_adoption_retains_source_and_terminal_reopen_is_observation_only() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner_from(ParentInputFault::LoseAdmissionAck, TestArtifacts::Disabled,
            crate::agent::subagent::ActiveAgentMessageSource::Human).await;
        let (sender, owner) = coordinator(&f.agent, None, None);
        let backend = ChannelBackend::for_coordinator_session(sender.clone(), f.parent.0.to_string());
        let response = f.server.expect_response("human recovery", InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Responses, TEXT),
            ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact(r#"{"value":"human-adopted-result"}"#, "test-model")));
        let sampled = f.server.request_count();
        assert_eq!(bounded(sender.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt,
            ActiveMessageSenderContext::HumanRoot { session_id: std::sync::Arc::from(f.parent.0.as_ref()) })).await,
            ActiveAgentMessageOutcome::Accepted { message_id: f.reference.message_id.clone() });
        let result = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.unwrap();
        assert!(matches!(result.status, SubagentSnapshotStatus::Completed { output, .. } if output.contains("human-adopted-result")));
        response.assert_satisfied();
        assert_eq!(f.server.request_count(), sampled + 1);
        let loaded = f.storage.load_session(&f.info).await.unwrap();
        let DeliveryProvenance::ParentMessage { admission } = &loaded.summary.completion_delivery.receiver.records[&f.reference.delivery_id].provenance else { panic!("human admission"); };
        assert_eq!(admission.source, crate::session::delivery_receiver::ParentMessageSource::Human);
        assert_eq!(admission.sender_session_id, f.parent.0.as_ref());
        let messages = loaded.chat_history.iter().filter(|item| matches!(item,
            xai_grok_sampling_types::conversation::ConversationItem::User(user)
                if user.synthetic_reason == xai_grok_sampling_types::conversation::SyntheticReason::ParentHumanMessage
                    && serde_json::to_string(user).unwrap().contains(TEXT))).count();
        assert_eq!(messages, 1);
        drop(backend);
        drop(owner);
        let history = std::fs::read(f.directory.join("chat_history.jsonl")).unwrap();
        let (sender, owner) = coordinator(&f.agent, None, None);
        assert_eq!(bounded(sender.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt,
            ActiveMessageSenderContext::HumanRoot { session_id: std::sync::Arc::from(f.parent.0.as_ref()) })).await,
            ActiveAgentMessageOutcome::RecoveryObserved { recovery_ref: f.reference, state: MessageRecoveryState::Succeeded });
        assert_eq!(history, std::fs::read(f.directory.join("chat_history.jsonl")).unwrap());
        assert_eq!(f.server.request_count(), sampled + 1);
        drop(owner);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_committed_wake_prefix_is_reconciled_without_double_trace_promotion() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner(ParentInputFault::ShutdownAfterWakeStart).await;
        let sampled = f.server.request_count();
        let (sender, owner) = coordinator(&f.agent, None, None);
        let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
        let mut response = f.server.expect_response_blocked("rearmed native publication", InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Responses, TEXT),
            ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact(r#"{"value":"prefix-adopted-result"}"#, "test-model")));
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await,
            ActiveAgentMessageOutcome::Accepted { message_id: f.reference.message_id.clone() });
        bounded(response.wait_blocked()).await;
        let promoted = f.storage.load_summary(&f.info).await.unwrap();
        assert_eq!(promoted.next_trace_turn, f.prior_next_turn + 1);
        assert_eq!(f.server.request_count(), sampled + 1);
        response.release();
        let result = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.unwrap();
        assert!(matches!(result.status, SubagentSnapshotStatus::Completed { output, .. } if output.contains("prefix-adopted-result")));
        response.assert_satisfied();
        assert!(matches!(f.storage.load_summary(&f.info).await.unwrap().completion_delivery.receiver.records[&f.reference.delivery_id].execution, DeliveryExecution::Succeeded));
        drop(owner);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_existing_start_reacquires_results_and_gate_without_republishing_or_recounting() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let mut f = lost_owner(ParentInputFault::ShutdownBeforeProjection).await;
        let published_meta = std::fs::read(f.artifacts.join("meta.json")).unwrap();
        while f.gateway.try_recv().is_ok() {}
        let sampled = f.server.request_count();
        let (sender, owner) = coordinator(&f.agent, None, None);
        let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
        let mut response = f.server.expect_response_blocked("existing native publication", InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Responses, TEXT),
            ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact(r#"{"value":"published-adopted-result"}"#, "test-model")));
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await,
            ActiveAgentMessageOutcome::Accepted { message_id: f.reference.message_id.clone() });
        bounded(response.wait_blocked()).await;
        assert_eq!(std::fs::read(f.artifacts.join("meta.json")).unwrap(), published_meta);
        assert_eq!(std::fs::read(f.artifacts.join("output.json")).unwrap(), f.prior_output);
        let promoted = f.storage.load_summary(&f.info).await.unwrap();
        assert_eq!(promoted.next_trace_turn, f.prior_next_turn + 1);
        assert_eq!(f.server.request_count(), sampled + 1);
        assert!(std::iter::from_fn(|| f.gateway.try_recv().ok()).all(|message| !matches!(message,
            xai_acp_lib::AcpClientMessage::ExtNotification(args) if args.request.params.get().contains("subagent_spawned"))));
        response.release();
        let result = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.unwrap();
        assert!(matches!(result.status, SubagentSnapshotStatus::Completed { output, .. } if output.contains("published-adopted-result")));
        response.assert_satisfied();
        let terminal = f.storage.load_summary(&f.info).await.unwrap();
        assert_eq!(terminal.next_trace_turn, f.prior_next_turn + 1);
        assert!(matches!(terminal.completion_delivery.receiver.records[&f.reference.delivery_id].execution, DeliveryExecution::Succeeded));
        drop(owner);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_human_observe_and_adopt_of_tool_uncertainty_keeps_original_agent_source() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner(ParentInputFault::LoseAdmissionAck).await;
        let (sender, owner) = coordinator(&f.agent, None, None);
        let backend = ChannelBackend::for_coordinator_session(sender.clone(), f.parent.0.to_string());
        let request = |action| acp::ExtRequest::new("x.ai/subagent/message/recover", std::sync::Arc::from(
            serde_json::value::to_raw_value(&serde_json::json!({"sessionId":f.parent.0.as_ref(),"action":action,"recoveryRef":f.reference})).unwrap()));
        let observed = bounded(crate::extensions::subagent_message::handle_recovery(&f.agent, &request("observe"), &sender)).await.unwrap();
        let observed: serde_json::Value = serde_json::from_str(observed.0.get()).unwrap();
        assert_eq!(observed["result"]["kind"], "recovery_observed");
        assert_eq!(observed["result"]["recoveryRef"], serde_json::to_value(&f.reference).unwrap());
        let sampled = f.server.request_count();
        let response = f.server.expect_response("human reauthorizes tool intent", InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Responses, TEXT),
            ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact(r#"{"value":"agent-source-preserved"}"#, "test-model")));
        let accepted = bounded(crate::extensions::subagent_message::handle_recovery(&f.agent, &request("adopt"), &sender)).await.unwrap();
        let accepted: serde_json::Value = serde_json::from_str(accepted.0.get()).unwrap();
        assert_eq!(accepted["result"]["kind"], "accepted");
        assert_eq!(accepted["result"]["messageId"], f.reference.message_id);
        let done = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.unwrap();
        assert!(matches!(done.status, SubagentSnapshotStatus::Completed { output, .. } if output.contains("agent-source-preserved")));
        response.assert_satisfied();
        assert_eq!(f.server.request_count(), sampled + 1);
        let loaded = f.storage.load_session(&f.info).await.unwrap();
        assert!(loaded.chat_history.iter().any(|item| matches!(item,
            xai_grok_sampling_types::conversation::ConversationItem::User(user)
                if user.synthetic_reason == xai_grok_sampling_types::conversation::SyntheticReason::AgentMessage
                    && serde_json::to_string(user).unwrap().contains(TEXT))));
        drop(owner);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_changed_management_recipe_is_not_replaced_with_current_defaults() {
    xai_test_utils::require_git!();
    tokio::task::LocalSet::new().run_until(async {
        let f = lost_owner(ParentInputFault::ShutdownAfterAdmission).await;
        let mut recipe = crate::agent::subagent::management_recipe::ChildManagementRecipe::load(&f.directory).unwrap().unwrap();
        recipe.role_instructions = Some("role changed after original admission".into());
        recipe.save(&f.directory).unwrap();
        let changed = std::fs::read(f.directory.join("child_management.json")).unwrap();
        let sampled = f.server.request_count();
        let (sender, owner) = coordinator(&f.agent, None, None);
        let backend = ChannelBackend::for_coordinator_session(sender, f.parent.0.to_string());
        assert_eq!(bounded(backend.recover_message(f.reference.clone(), MessageRecoveryAction::Adopt)).await,
            ActiveAgentMessageOutcome::AdmissionUncertain { recovery_ref: f.reference.clone() });
        let preserved = bounded(backend.query(&f.reference.receiver.session_id, true, Some(30_000), None)).await.unwrap();
        assert!(matches!(preserved.status, SubagentSnapshotStatus::Completed { output, .. } if output.as_str() == f.prior_result.output.as_ref()));
        assert_eq!(sampled, f.server.request_count());
        assert_eq!(std::fs::read(f.directory.join("child_management.json")).unwrap(), changed);
        assert_eq!(std::fs::read(f.artifacts.join("meta.json")).unwrap(), f.prior_meta);
        assert_eq!(std::fs::read(f.artifacts.join("output.json")).unwrap(), f.prior_output);
        assert!(matches!(f.storage.load_summary(&f.info).await.unwrap().completion_delivery.receiver.records[&f.reference.delivery_id].execution, DeliveryExecution::Queued { .. }));
        drop(owner);
    }).await;
}
