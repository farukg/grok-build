use super::*;
use crate::session::storage::{StorageAdapter, jsonl::JsonlStorageAdapter};
use xai_grok_tools::implementations::grok_build::task::{
    backend::{ChannelBackend, SubagentBackend},
    coordinator::{ChildControl, ChildRunner, ChildRunRequest, ChildRunOutput, StartedChild,
        SendBoxFuture, SubagentProgress, SubagentCoordinator, ChildCompletion,
        CompletionPreparation, TerminalCommitResult, TerminalVisibilityResult},
    types::{ActiveAgentMessageRequest, ActiveAgentMessageOutcome, SubagentRequest,
        SubagentOwner, SubagentValidateTypeOutcome, SubagentDescribeOutcome},
};

#[path = "parent_message_boundary_tests.rs"]
mod boundary_tests;
#[path = "parent_message_exit_tests.rs"]
mod exit_tests;

type HeldDelivery = (ActiveAgentMessageDelivery, oneshot::Sender<ActiveMessageAdmission>);

#[derive(Clone)]
struct ClaimReceiverControl(mpsc::UnboundedSender<HeldDelivery>);

impl ChildControl for ClaimReceiverControl {
    type ProgressFuture = std::future::Ready<SubagentProgress>;

    fn progress(&self) -> Self::ProgressFuture {
        std::future::ready(Default::default())
    }

    fn send_active_message(&self, delivery: ActiveAgentMessageDelivery) -> SendBoxFuture<ActiveMessageAdmission> {
        let (reply, response) = oneshot::channel();
        self.0.send((delivery, reply)).unwrap();
        Box::pin(async move { response.await.unwrap_or(ActiveMessageAdmission::ChannelClosed) })
    }

    fn cancel(&self) {}
}

struct ClaimReceiverRunner {
    control: ClaimReceiverControl,
    started: mpsc::UnboundedSender<()>,
}

impl ChildRunner for ClaimReceiverRunner {
    type Control = ClaimReceiverControl;
    type RootControl = xai_grok_tools::implementations::grok_build::task::root_control::NoRootControl;
    type CompletionData = ();
    type RunFuture = SendBoxFuture<ChildRunOutput<()>>;
    type ValidateFuture = SendBoxFuture<SubagentValidateTypeOutcome>;
    type DescribeFuture = SendBoxFuture<SubagentDescribeOutcome>;

    fn run(&self, run: ChildRunRequest<Self::Control>) -> Self::RunFuture {
        let control = self.control.clone();
        let started = self.started.clone();
        Box::pin(async move {
            assert!(run.reporter.started(StartedChild {
                child_session_id: run.request.id.clone(), persona: None, resumed_from: None,
                child_cwd: String::new(), worktree_path: None, effective_model_id: "test".into(),
                definition_background: false, control,
            }).await);
            started.send(()).unwrap();
            std::future::pending().await
        })
    }

    fn validate_type(&self, _: String, _: String) -> Self::ValidateFuture {
        Box::pin(std::future::pending())
    }

    fn describe_type(&self, _: String, _: Option<String>, _: String) -> Self::DescribeFuture {
        Box::pin(std::future::pending())
    }

    fn supports_wake(&self) -> bool { true }

    fn on_completed(&self, _: CompletionPreparation<'_, ()>, published: Box<dyn FnOnce(TerminalCommitResult) + Send>) {
        published(TerminalCommitResult::Committed);
    }

    fn on_published(&self, _: ChildCompletion<()>, published: Box<dyn FnOnce(TerminalVisibilityResult) + Send>) {
        published(TerminalVisibilityResult::Published);
    }
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(15), future).await.unwrap()
}

struct Producer {
    delivery: ActiveAgentMessageDelivery,
    reply: oneshot::Sender<ActiveMessageAdmission>,
    outcome: tokio::task::JoinHandle<ActiveAgentMessageOutcome>,
    cancellation: tokio_util::sync::CancellationToken,
    _coordinator: tokio_util::task::AbortOnDropHandle<()>,
    _spawn: tokio_util::task::AbortOnDropHandle<()>,
}

async fn producer() -> Producer {
    producer_with_operation(ActiveAgentMessageOperation::Steer).await
}

async fn producer_with_operation(operation: ActiveAgentMessageOperation) -> Producer {
    let (sender, receiver) = SubagentCoordinator::<ClaimReceiverRunner>::channel();
    let backend = ChannelBackend::for_coordinator_session(sender, "owning-parent");
    let (deliveries, mut held) = mpsc::unbounded_channel();
    let (started, mut ready) = mpsc::unbounded_channel();
    let coordinator = SubagentCoordinator::from_channel(receiver, ClaimReceiverRunner {
        control: ClaimReceiverControl(deliveries), started,
    }, Default::default());
    let coordinator = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(coordinator.run()));
    let cancellation = tokio_util::sync::CancellationToken::new();
    let request = SubagentRequest {
        id: "receiver-child".into(), prompt: String::new(), description: String::new(),
        subagent_type: "general-purpose".into(), parent_session_id: "owning-parent".into(),
        parent_prompt_id: None, resume_from: None, cwd: None, runtime_overrides: Default::default(),
        run_in_background: true, surface_completion: false, await_to_completion: false,
        fork_context: false, owner: SubagentOwner::Task, cancel_token: cancellation.clone(),
        spawn_root: Default::default(), tool_call_id: None,
    };
    let spawn = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local({
        let backend = backend.clone();
        async move { let _ = backend.spawn(request, None, None).await; }
    }));
    bounded(ready.recv()).await.unwrap();
    let outcome = tokio::task::spawn_local(async move {
        backend.send_active_message(ActiveAgentMessageRequest::try_new_with_operation(
            "receiver-child", "claim-bound text stays behind the tool boundary", operation,
        ).unwrap()).await
    });
    let (delivery, reply) = bounded(held.recv()).await.unwrap();
    Producer { delivery, reply, outcome, cancellation, _coordinator: coordinator, _spawn: spawn }
}

type HeldAck = (
    crate::session::delivery_receiver::ParentMessageAdmission,
    std::io::Result<crate::session::delivery_receiver::ParentAdmissionAck>,
    oneshot::Sender<std::io::Result<crate::session::delivery_receiver::ParentAdmissionAck>>,
);

#[derive(Clone, Copy)]
enum ReceiverBarrier { Forward, HoldNext }

#[derive(Clone, Copy)]
enum ReceiverSettlementAck { Forward, Lose }

#[derive(Debug, PartialEq, Eq)]
enum SettlementObservation { Committed, Fault(crate::session::storage::AtomicWriteFault), OtherError }

type HeldProjection = (xai_chat_state::InputProjection,
    Result<xai_chat_state::CompletionProjectionAck, xai_chat_state::CompletionProjectionError>,
    oneshot::Sender<Result<xai_chat_state::CompletionProjectionAck, xai_chat_state::CompletionProjectionError>>);

struct ReceiverFixture {
    actor: Arc<SessionActor>,
    storage: Arc<JsonlStorageAdapter>,
    acknowledgements: mpsc::UnboundedReceiver<HeldAck>,
    events: mpsc::UnboundedReceiver<SessionEvent>,
    settlements: mpsc::UnboundedReceiver<()>,
    settlement_results: mpsc::UnboundedReceiver<SettlementObservation>,
    projections: mpsc::UnboundedReceiver<HeldProjection>,
    barriers: mpsc::UnboundedReceiver<(std::io::Result<()>, oneshot::Sender<std::io::Result<()>>)>,
    _directory: tempfile::TempDir,
    _persistence: tokio_util::task::AbortOnDropHandle<()>,
    _proxy: tokio_util::task::AbortOnDropHandle<()>,
}

async fn receiver() -> ReceiverFixture {
    let directory = tempfile::tempdir().unwrap();
    let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
    receiver_with_storage(directory, storage).await
}

async fn receiver_with_storage(directory: tempfile::TempDir, storage: JsonlStorageAdapter) -> ReceiverFixture {
    receiver_with_cuts(directory, storage, ReceiverBarrier::Forward).await
}

async fn receiver_with_cuts(directory: tempfile::TempDir, storage: JsonlStorageAdapter, barrier: ReceiverBarrier) -> ReceiverFixture {
    receiver_with_exit_cuts(directory, storage, barrier, ReceiverSettlementAck::Forward, ReceiverBarrier::Forward).await
}

async fn receiver_with_exit_cuts(directory: tempfile::TempDir, storage: JsonlStorageAdapter, mut barrier: ReceiverBarrier,
    settlement_ack: ReceiverSettlementAck, mut projection_ack: ReceiverBarrier) -> ReceiverFixture {
    use crate::session::chat_persistence::ChannelChatPersistence;
    let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
    let storage = Arc::new(storage);
    storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
    let persistence = crate::session::persistence::durable_update_tests::test_actor(info, storage.clone());
    let (proxy, mut requests) = mpsc::unbounded_channel();
    let (held, acknowledgements) = mpsc::unbounded_channel();
    let (settling, settlements) = mpsc::unbounded_channel();
    let (held_barriers, barriers) = mpsc::unbounded_channel();
    let (settled, settlement_results) = mpsc::unbounded_channel();
    let (held_projections, projections) = mpsc::unbounded_channel();
    let real = persistence.handle.tx.clone();
    let proxy_task = tokio::task::spawn_local(async move {
        while let Some(message) = requests.recv().await {
            match message {
                PersistenceMsg::AdmitParentMessage { admission, authority, respond_to } => {
                    let (reply, response) = oneshot::channel();
                    real.send(PersistenceMsg::AdmitParentMessage { admission: admission.clone(), authority, respond_to: reply }).unwrap();
                    let Ok(result) = response.await else { continue; };
                    let _ = held.send((admission, result, respond_to));
                }
                PersistenceMsg::FlushAndAck { respond_to } if matches!(barrier, ReceiverBarrier::HoldNext) => {
                    barrier = ReceiverBarrier::Forward;
                    let (reply, response) = oneshot::channel();
                    real.send(PersistenceMsg::FlushAndAck { respond_to: reply }).unwrap();
                    let result = response.await.unwrap();
                    let _ = held_barriers.send((result, respond_to));
                }
                PersistenceMsg::CancelParentMessages { message_ids, respond_to } => {
                    let (reply, response) = oneshot::channel();
                    real.send(PersistenceMsg::CancelParentMessages { message_ids, respond_to: reply }).unwrap();
                    let _ = settling.send(());
                    let result = response.await.unwrap();
                    let observation = match &result {
                        Ok(()) => SettlementObservation::Committed,
                        Err(error) => error.get_ref().and_then(|error| error.downcast_ref::<crate::session::storage::AtomicWriteFailure>())
                            .map_or(SettlementObservation::OtherError, |failure| SettlementObservation::Fault(failure.fault)),
                    };
                    let _ = settled.send(observation);
                    match settlement_ack {
                        ReceiverSettlementAck::Forward => { let _ = respond_to.send(result); }
                        ReceiverSettlementAck::Lose => drop(respond_to),
                    }
                }
                PersistenceMsg::AppendInputAndAck { projection: projection @ xai_chat_state::InputProjection::Delivery(_), admission, respond_to }
                    if matches!(projection_ack, ReceiverBarrier::HoldNext) => {
                    projection_ack = ReceiverBarrier::Forward;
                    let (reply, response) = oneshot::channel();
                    real.send(PersistenceMsg::AppendInputAndAck { projection: projection.clone(), admission, respond_to: reply }).unwrap();
                    let result = response.await.unwrap();
                    let _ = held_projections.send((projection, result, respond_to));
                }
                other => real.send(other).unwrap(),
            }
        }
    });
    let (gateway, gateway_rx) = mpsc::unbounded_channel();
    super::super::support::drain_gateway(gateway_rx);
    let (mut actor, events) = super::super::support::create_test_actor_with_chat_persistence(
        0, 256_000, 85, gateway, proxy.clone(),
        Box::new(ChannelChatPersistence::new(proxy.clone())),
    ).await;
    actor.notifications.persistence_tx = proxy;
    let actor = Arc::new(actor);
    tests::start_turn(&actor, "held-tool-turn").await;
    ReceiverFixture { actor, storage, acknowledgements, events, settlements, settlement_results, projections, barriers, _directory: directory,
        _persistence: tokio_util::task::AbortOnDropHandle::new(persistence.task),
        _proxy: tokio_util::task::AbortOnDropHandle::new(proxy_task) }
}

fn dispatch(
    actor: Arc<SessionActor>, delivery: ActiveAgentMessageDelivery,
    reply: oneshot::Sender<ActiveMessageAdmission>,
) -> (tokio::task::JoinHandle<()>, mpsc::Receiver<crate::agent::subagent::PromptTurnReceipt>) {
    let (receipt_sink, receipts) = mpsc::channel(1);
    let (completion, _completion) = mpsc::unbounded_channel();
    let task = tokio::task::spawn_local(async move {
        actor.admit_parent_agent_message(delivery, receipt_sink,
            xai_grok_telemetry::TelemetryCtx::new("owning-parent".into(), Arc::new(tokio::sync::Mutex::new(0))),
            reply, completion).await;
    });
    (task, receipts)
}

async fn persisted_bindings(receiver: &ReceiverFixture) -> Vec<xai_chat_state::DeliveryBinding> {
    let path = receiver._directory.path().join(crate::session::storage::CHAT_HISTORY_FILE);
    let text = match tokio::fs::read_to_string(path).await {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => panic!("read durable history: {error}"),
    };
    text.lines().filter_map(|line| {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        row.get("_session_delivery").map(|binding| serde_json::from_value(binding.clone()).unwrap())
    }).collect()
}

#[derive(Clone, Copy, Debug)]
enum ObservationCut { Deadline, Cancellation, ReceiverDrop, LostAck, ReceiptClosed, Complete }

#[tokio::test(flavor = "current_thread")]
async fn real_claim_and_persistence_ack_cuts_preserve_replayable_intent() {
    tokio::task::LocalSet::new().run_until(async {
        for cut in [ObservationCut::Deadline, ObservationCut::Cancellation, ObservationCut::ReceiverDrop, ObservationCut::LostAck, ObservationCut::ReceiptClosed, ObservationCut::Complete] {
            let mut receiver = receiver().await;
            let Producer { delivery, reply, outcome, cancellation, _coordinator, _spawn } = producer().await;
            let original = delivery.message().clone();
            let expected_parent = xai_chat_state::ParentTask::Instance {
                id: receiver.actor.state.lock().await.running_task.as_ref().unwrap().identity.as_ref().clone(),
            };
            let (admitting, mut receipts) = dispatch(receiver.actor.clone(), delivery, reply);
            let (admission, proof, ack) = bounded(receiver.acknowledgements.recv()).await.unwrap();
            assert!(proof.is_ok(), "real persistence must commit before the held ACK");
            assert_eq!(admission.message_id, original.message_id);
            assert_eq!(admission.sender_session_id, original.sender_session_id);
            assert_eq!(admission.text, original.text.as_ref());
            assert_eq!(admission.sender_session_id, "owning-parent");
            assert_eq!(admission.text, "claim-bound text stays behind the tool boundary");
            assert_eq!(admission.source, crate::session::delivery_receiver::ParentMessageSource::Agent);
            match &admission.effective {
                crate::session::delivery_receiver::ParentMessageOperation::Steer { prompt_id, parent_task } => {
                    assert_eq!(prompt_id, "held-tool-turn");
                    assert_eq!(parent_task, &expected_parent);
                }
                other => panic!("wrong effective operation: {other:?}"),
            }
            assert!(!admitting.is_finished());
            assert!(!outcome.is_finished());
            assert!(matches!(receipts.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
            let reopened = JsonlStorageAdapter::with_explicit_session_dir(receiver._directory.path().to_path_buf());
            let recovered = reopened.load_session(&receiver.actor.session_info).await.unwrap();
            let record = &recovered.summary.completion_delivery.receiver.records[&admission.delivery_id()];
            assert!(matches!(&record.execution, crate::session::delivery_receiver::DeliveryExecution::Queued { payload, prompt_id } if payload == &admission.text && prompt_id == &admission.prompt_id));
            assert!(matches!(&record.provenance, crate::session::delivery_receiver::DeliveryProvenance::ParentMessage { admission: stored } if stored == &admission));
            assert!(recovered.chat_history.iter().all(|item| item.text_content() != admission.text));
            assert!(persisted_bindings(&receiver).await.is_empty());
            match cut {
                ObservationCut::Deadline => {
                    assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::AdmissionUncertain);
                    ack.send(proof).unwrap();
                }
                ObservationCut::Cancellation => {
                    cancellation.cancel();
                    assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::AdmissionUncertain);
                    ack.send(proof).unwrap();
                }
                ObservationCut::ReceiverDrop => {
                    admitting.abort();
                    assert!(bounded(admitting).await.unwrap_err().is_cancelled());
                    drop(ack);
                    drop(proof);
                    assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::AdmissionUncertain);
                    assert!(receipts.recv().await.is_none());
                    assert!(receiver.actor.state.lock().await.message_delivery.is_empty());
                    receiver._persistence.abort();
                    assert!(bounded(&mut receiver._persistence).await.unwrap_err().is_cancelled());
                    let (_lease, recovered) = reopened.claim_receiver(&receiver.actor.session_info).await.unwrap();
                    assert!(matches!(recovered.completion_delivery.receiver.records[&admission.delivery_id()].execution, crate::session::delivery_receiver::DeliveryExecution::Queued { .. }));
                    assert!(reopened.load_session(&receiver.actor.session_info).await.unwrap().chat_history.iter().all(|item| item.text_content() != admission.text));
                    assert!(persisted_bindings(&receiver).await.is_empty());
                    drop(_lease);
                    boundary_tests::execute_recovered_literal(receiver._directory.path(), &original.message_id).await;
                    continue;
                }
                ObservationCut::LostAck => {
                    drop(ack);
                    drop(proof);
                    assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::AdmissionUncertain);
                }
                ObservationCut::ReceiptClosed => {
                    receipts.close();
                    ack.send(proof).unwrap();
                    assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::AdmissionUncertain);
                }
                ObservationCut::Complete => {
                    ack.send(proof).unwrap();
                    assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::Accepted { message_id: original.message_id.clone() });
                }
            }
            bounded(admitting).await.unwrap();
            match cut {
                ObservationCut::LostAck => {
                    assert!(receipts.recv().await.is_none());
                    assert!(receiver.actor.state.lock().await.message_delivery.is_empty());
                    receiver._persistence.abort();
                    assert!(bounded(&mut receiver._persistence).await.unwrap_err().is_cancelled());
                    let (_lease, recovered) = reopened.claim_receiver(&receiver.actor.session_info).await.unwrap();
                    assert!(matches!(recovered.completion_delivery.receiver.records[&admission.delivery_id()].execution, crate::session::delivery_receiver::DeliveryExecution::Queued { .. }));
                    let history = reopened.load_session(&receiver.actor.session_info).await.unwrap().chat_history;
                    assert!(history.iter().all(|item| item.text_content() != admission.text));
                    assert!(persisted_bindings(&receiver).await.is_empty());
                    drop(_lease);
                    boundary_tests::execute_recovered_literal(receiver._directory.path(), &original.message_id).await;
                }
                ObservationCut::ReceiptClosed => {
                    assert!(!receiver.actor.state.lock().await.message_delivery.is_empty());
                    assert!(persisted_bindings(&receiver).await.is_empty());
                    let recovered = reopened.load_session(&receiver.actor.session_info).await.unwrap();
                    assert!(matches!(recovered.summary.completion_delivery.receiver.records[&admission.delivery_id()].execution, crate::session::delivery_receiver::DeliveryExecution::Queued { .. }));
                }
                ObservationCut::Deadline | ObservationCut::Cancellation | ObservationCut::Complete => {
                    let mut receipt = bounded(receipts.recv()).await.unwrap();
                    assert_eq!(receipt.prompt_id, format!("parent-message-{}", original.message_id));
                    assert!(matches!(receipt.result.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                    assert!(receiver.actor.chat_state_handle.snapshot().await.unwrap().conversation.iter().all(|item| item.text_content() != admission.text));
                    assert!(bounded(receiver.actor.drain_parent_messages_at_safe_point()).await);
                    assert!(!bounded(receiver.actor.drain_parent_messages_at_safe_point()).await);
                    let recovered = receiver.storage.load_session(&receiver.actor.session_info).await.unwrap();
                    assert_eq!(recovered.chat_history.iter().filter(|item| item.text_content() == admission.text).count(), 1);
                    let bindings = persisted_bindings(&receiver).await;
                    assert_eq!(bindings.len(), 1);
                    assert_eq!(bindings[0].delivery_id, admission.delivery_id());
                    assert_eq!(bindings[0].parent_task, expected_parent);
                    assert_eq!(bindings[0].turn_id, "held-tool-turn");
                    use sha2::Digest;
                    assert_eq!(bindings[0].payload_digest, format!("{:x}", sha2::Sha256::digest(b"claim-bound text stays behind the tool boundary")));
                    let item = recovered.chat_history.iter().find(|item| item.text_content() == "claim-bound text stays behind the tool boundary").unwrap();
                    boundary_tests::assert_agent_model_projection(item);
                    let ConversationItem::User(user) = item else { panic!("parent source must project as user input"); };
                    assert_eq!(user.synthetic_reason, xai_grok_sampling_types::SyntheticReason::AgentMessage);
                    receiver.actor.settle_all_parent_messages(TerminalCause::HardTeardown).await.unwrap();
                    assert!(bounded(receipt.result).await.unwrap().is_err());
                }
                ObservationCut::ReceiverDrop => unreachable!(),
            }
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn revoked_before_receiver_claim_never_dispatches_or_persists() {
    tokio::task::LocalSet::new().run_until(async {
        let mut receiver = receiver().await;
        let Producer { delivery, reply, outcome, cancellation: _, _coordinator, _spawn } = producer().await;
        let message_id = delivery.message().message_id.clone();
        assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::NotAcceptedBeforeDeadline);
        let (admitting, mut receipts) = dispatch(receiver.actor.clone(), delivery, reply);
        bounded(admitting).await.unwrap();
        assert!(matches!(receiver.acknowledgements.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        assert!(receipts.recv().await.is_none());
        assert!(receiver.actor.state.lock().await.message_delivery.is_empty());
        let state = receiver.storage.read_completion_state(&receiver.actor.session_info).await.unwrap();
        assert!(!state.receiver.records.contains_key(&crate::session::delivery_receiver::parent_message_delivery_id(&message_id)));
        assert!(persisted_bindings(&receiver).await.is_empty());
    }).await;
}

#[derive(Clone, Copy, Debug)]
enum AdmissionFault { Message, Sender, Source, Text, Operation, TaskBinding }

#[tokio::test(flavor = "current_thread")]
async fn real_claim_storage_refuses_altered_admission_without_positive_settlement() {
    use crate::session::delivery_receiver::{ParentAdmissionAuthority, ParentMessageAdmission, ParentMessageOperation, ParentMessageSource};
    tokio::task::LocalSet::new().run_until(async {
        for fault in [AdmissionFault::Message, AdmissionFault::Sender, AdmissionFault::Source, AdmissionFault::Text, AdmissionFault::Operation, AdmissionFault::TaskBinding] {
            let directory = tempfile::tempdir().unwrap();
            let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
            let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
            storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
            let (lease, _) = storage.claim_receiver(&info).await.unwrap();
            let Producer { delivery, reply, outcome, cancellation: _, _coordinator, _spawn } = producer().await;
            let message = delivery.message().clone();
            let mut admission = ParentMessageAdmission {
                prompt_id: format!("parent-message-{}", message.message_id), message_id: message.message_id,
                sender_session_id: message.sender_session_id, text: message.text.to_string(),
                source: ParentMessageSource::Agent,
                effective: ParentMessageOperation::Steer { prompt_id: "held-tool-turn".into(), parent_task: xai_chat_state::ParentTask::Instance { id: "exact-live-instance".into() } },
            };
            match fault {
                AdmissionFault::Message => admission.message_id.push_str("-altered"),
                AdmissionFault::Sender => admission.sender_session_id.push_str("-altered"),
                AdmissionFault::Source => admission.source = ParentMessageSource::Human,
                AdmissionFault::Text => admission.text.push_str("-altered"),
                AdmissionFault::Operation => admission.effective = ParentMessageOperation::Interject { prompt_id: "held-tool-turn".into(), parent_task: xai_chat_state::ParentTask::Instance { id: "exact-live-instance".into() } },
                AdmissionFault::TaskBinding => admission.effective = ParentMessageOperation::Steer { prompt_id: "held-tool-turn".into(), parent_task: xai_chat_state::ParentTask::Unknown },
            }
            let authority = ParentAdmissionAuthority::Coordinator(delivery.claim_admission().unwrap());
            let error = storage.admit_parent_message(&info, &admission, authority,
                crate::session::receiver_execution::ReceiverAccess::Owner(&lease)).await.unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{fault:?}");
            reply.send(ActiveMessageAdmission::ChannelClosed).unwrap();
            assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::AdmissionUncertain);
            let recovered = storage.load_session(&info).await.unwrap();
            assert!(recovered.summary.completion_delivery.receiver.records.is_empty());
            assert!(recovered.chat_history.is_empty());
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn active_steer_admission_waits_for_typed_durable_receipt_without_publishing_text() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, _gateway_rx) = mpsc::unbounded_channel();
        let (capture, mut persistence) = mpsc::unbounded_channel();
        let (mut actor, _) = super::super::support::create_test_actor_with_chat_persistence(
            0, 256_000, 85, gateway, capture.clone(), Box::new(xai_chat_state::NullChatPersistence),
        ).await;
        actor.notifications.persistence_tx = capture;
        let actor = Arc::new(actor);
        tests::start_turn(&actor, "held-tool-turn").await;
        let (receipt_sink, mut receipts) = mpsc::channel(1);
        let (respond_to, mut response) = oneshot::channel();
        let (completion_tx, _completion_rx) = mpsc::unbounded_channel();
        let admission_task = tokio::task::spawn_local({
            let actor = actor.clone();
            async move {
                actor.admit_parent_agent_message_for_test(
                    tests::message_with_text("durable-steer", "not visible before tool result"),
                    ActiveAgentMessageSource::Human, ActiveAgentMessageOperation::Steer,
                    receipt_sink, respond_to, completion_tx,
                ).await;
            }
        });
        let (admission, authority, ack) = loop {
            if let PersistenceMsg::AdmitParentMessage { admission, authority, respond_to } = tests::await_with_timeout(persistence.recv()).await.unwrap() {
                break (admission, authority, respond_to);
            }
        };
        assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
        assert!(matches!(receipts.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        assert_eq!(admission.source, crate::session::delivery_receiver::ParentMessageSource::Human);
        assert!(matches!(&admission.effective, crate::session::delivery_receiver::ParentMessageOperation::Steer { prompt_id, parent_task: xai_chat_state::ParentTask::Instance { .. } } if prompt_id == "held-tool-turn"));
        let directory = tempfile::tempdir().unwrap();
        let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
        storage.init_session(&actor.session_info, acp::ModelId::new("test")).await.unwrap();
        let (lease, _) = storage.claim_receiver(&actor.session_info).await.unwrap();
        let proof = storage.admit_parent_message(&actor.session_info, &admission, authority, crate::session::receiver_execution::ReceiverAccess::Owner(&lease)).await.unwrap();
        let restored = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
        let state = restored.read_completion_state(&actor.session_info).await.unwrap();
        assert!(state.receiver.records.contains_key(&admission.delivery_id()));
        assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)), "even committed storage cannot release the producer before its ACK");
        ack.send(Ok(proof)).unwrap();
        tests::await_with_timeout(admission_task).await.unwrap();
        assert_eq!(tests::await_with_timeout(response).await.unwrap(), ActiveMessageAdmission::Admitted);
        let receipt = receipts.recv().await.unwrap();
        let conversation = actor.chat_state_handle.snapshot().await.unwrap();
        assert!(conversation.conversation.iter().all(|item| item.text_content() != admission.text));
        drop(receipt);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn failed_or_lost_parent_admission_ack_never_reports_admitted() {
    tokio::task::LocalSet::new().run_until(async {
        for lose_ack in [false, true] {
            let (gateway, _gateway_rx) = mpsc::unbounded_channel();
            let (capture, mut persistence) = mpsc::unbounded_channel();
            let (mut actor, _) = super::super::support::create_test_actor_with_chat_persistence(
                0, 256_000, 85, gateway, capture.clone(), Box::new(xai_chat_state::NullChatPersistence),
            ).await;
            actor.notifications.persistence_tx = capture;
            let actor = Arc::new(actor);
            tests::start_turn(&actor, "held-tool-turn").await;
            let (receipt_sink, mut receipts) = mpsc::channel(1);
            let (respond_to, mut response) = oneshot::channel();
            let (completion_tx, _completion_rx) = mpsc::unbounded_channel();
            let task = tokio::task::spawn_local({
                let actor = actor.clone();
                async move { actor.admit_parent_agent_message_for_test(
                    tests::message_with_text("failed-steer", "must remain invisible"),
                    ActiveAgentMessageSource::Agent, ActiveAgentMessageOperation::Steer,
                    receipt_sink, respond_to, completion_tx,
                ).await; }
            });
            let ack = loop {
                if let PersistenceMsg::AdmitParentMessage { respond_to, .. } = tests::await_with_timeout(persistence.recv()).await.unwrap() { break respond_to; }
            };
            assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            if lose_ack { drop(ack); } else { ack.send(Err(std::io::Error::other("injected admission failure"))).unwrap(); }
            tests::await_with_timeout(task).await.unwrap();
            assert_eq!(tests::await_with_timeout(response).await.unwrap(), ActiveMessageAdmission::ChannelClosed);
            assert!(receipts.recv().await.is_none());
            assert!(actor.state.lock().await.message_delivery.is_empty());
        }
    }).await;
}
