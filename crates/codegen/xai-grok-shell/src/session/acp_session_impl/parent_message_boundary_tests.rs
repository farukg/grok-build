use super::*;
use crate::session::delivery_receiver::{DeliveryExecution, DeliveryProvenance, ParentMessageOperation, ParentMessageSource, ReceiverResume};
use std::sync::{Condvar, Mutex};

#[derive(Clone, Copy)]
enum BlockingState { Held, Released }

struct BlockingCut(Arc<(Mutex<BlockingState>, Condvar)>);

impl BlockingCut {
    fn release(&self) {
        let (state, wake) = &*self.0;
        *state.lock().unwrap() = BlockingState::Released;
        wake.notify_all();
    }
}

impl Drop for BlockingCut {
    fn drop(&mut self) { self.release(); }
}

pub(super) async fn stop_persistence(receiver: &mut ReceiverFixture) {
    receiver._persistence.abort();
    assert!(bounded(&mut receiver._persistence).await.unwrap_err().is_cancelled());
}

async fn wait_for_detached_write(directory: &std::path::Path) {
    let path = directory.join("delivery-execution.lock");
    bounded(tokio::task::spawn_blocking(move || {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(path).unwrap();
        fs2::FileExt::lock_exclusive(&file).unwrap();
        drop(file);
    })).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn detached_pre_append_admission_excludes_replacement_until_literal_intent_is_durable() {
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let gate = BlockingCut(Arc::new((Mutex::new(BlockingState::Held), Condvar::new())));
        let held = gate.0.clone();
        let entered = Arc::new(tokio::sync::Notify::new());
        let signal = entered.clone();
        let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf())
            .with_parent_admission_probe(move || {
                signal.notify_one();
                let (state, wake) = &*held;
                let mut state = state.lock().unwrap();
                while matches!(*state, BlockingState::Held) {
                    state = wake.wait(state).unwrap();
                }
                Ok(())
            });
        let mut receiver = receiver_with_storage(directory, storage).await;
        let Producer { delivery, reply, outcome, _coordinator, _spawn, .. } = producer().await;
        let message_id = delivery.message().message_id.clone();
        let original_task = receiver.actor.state.lock().await.running_task.as_ref().unwrap().identity.as_ref().clone();
        let (admitting, mut receipts) = dispatch(receiver.actor.clone(), delivery, reply);
        bounded(entered.notified()).await;
        admitting.abort();
        assert!(bounded(admitting).await.unwrap_err().is_cancelled());
        stop_persistence(&mut receiver).await;
        let reopened = JsonlStorageAdapter::with_explicit_session_dir(receiver._directory.path().to_path_buf());
        let error = match bounded(reopened.claim_receiver(&receiver.actor.session_info)).await {
            Ok(_) => panic!("replacement recovered before the old detached admission completed"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert!(receipts.recv().await.is_none());
        assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::AdmissionUncertain);
        gate.release();
        wait_for_detached_write(receiver._directory.path()).await;
        let (lease, recovered) = reopened.claim_receiver(&receiver.actor.session_info).await.unwrap();
        let id = crate::session::delivery_receiver::parent_message_delivery_id(&message_id);
        let record = &recovered.completion_delivery.receiver.records[&id];
        assert!(matches!(&record.execution, DeliveryExecution::Queued { payload, prompt_id }
            if payload == "claim-bound text stays behind the tool boundary" && prompt_id == &format!("parent-message-{message_id}")));
        let DeliveryProvenance::ParentMessage { admission } = &record.provenance else { panic!("lost parent provenance"); };
        assert_eq!(admission.sender_session_id, "owning-parent");
        assert_eq!(admission.source, ParentMessageSource::Agent);
        assert_eq!(admission.effective, ParentMessageOperation::Steer {
            prompt_id: "held-tool-turn".into(), parent_task: xai_chat_state::ParentTask::Instance { id: original_task },
        });
        assert!(persisted_bindings(&receiver).await.is_empty());
        drop(lease);
        execute_recovered_literal(receiver._directory.path(), &message_id).await;
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn actual_admission_atomic_write_and_sync_faults_leave_producer_uncertain() {
    use crate::session::storage::{AtomicWriteFault, fail_next_atomic_write};
    tokio::task::LocalSet::new().run_until(async {
        for fault in [AtomicWriteFault::Write, AtomicWriteFault::Sync, AtomicWriteFault::ParentSync] {
            let directory = tempfile::tempdir().unwrap();
            let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf())
                .with_parent_admission_probe(move || { fail_next_atomic_write(fault); Ok(()) });
            let mut receiver = receiver_with_storage(directory, storage).await;
            let Producer { delivery, reply, outcome, _coordinator, _spawn, .. } = producer().await;
            let message_id = delivery.message().message_id.clone();
            let (admitting, mut receipts) = dispatch(receiver.actor.clone(), delivery, reply);
            let (_, result, ack) = bounded(receiver.acknowledgements.recv()).await.unwrap();
            assert!(result.is_err(), "actual atomic admission must fail before creating a proof");
            let failure = result.as_ref().unwrap_err().get_ref().unwrap()
                .downcast_ref::<crate::session::storage::AtomicWriteFailure>().unwrap();
            assert_eq!(failure.fault, fault, "an unrelated earlier failure cannot satisfy the cut");
            ack.send(result).unwrap();
            bounded(admitting).await.unwrap();
            assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::AdmissionUncertain);
            assert!(receipts.recv().await.is_none());
            assert!(receiver.actor.state.lock().await.message_delivery.is_empty());
            stop_persistence(&mut receiver).await;
            let reopened = JsonlStorageAdapter::with_explicit_session_dir(receiver._directory.path().to_path_buf());
            let (lease, summary) = reopened.claim_receiver(&receiver.actor.session_info).await.unwrap();
            assert!(reopened.load_session(&receiver.actor.session_info).await.unwrap().chat_history.is_empty());
            match fault {
                AtomicWriteFault::Write | AtomicWriteFault::Sync => assert!(summary.completion_delivery.receiver.records.is_empty()),
                AtomicWriteFault::ParentSync => {
                    assert!(matches!(summary.completion_delivery.receiver.records[&format!("parent-message/{message_id}")].execution, DeliveryExecution::Queued { .. }));
                    drop(lease);
                    execute_recovered_literal(receiver._directory.path(), &message_id).await;
                }
            }
        }
    }).await;
}

pub(super) async fn actual_shutdown(receiver: &mut ReceiverFixture, kind: crate::session::ShutdownKind) {
    let (_, empty) = mpsc::unbounded_channel();
    let events = std::mem::replace(&mut receiver.events, empty);
    let (commands, command_rx) = mpsc::unbounded_channel();
    let (_chat, chat_rx) = mpsc::unbounded_channel();
    commands.send(SessionCommand::Shutdown(kind)).unwrap();
    bounded(super::super::super::run_session(
        receiver.actor.clone(), command_rx, chat_rx, events, None,
        Arc::new(parking_lot::Mutex::new(xai_grok_workspace::file_system::CodebaseIndexManager::new())),
        std::path::PathBuf::from("/tmp"), crate::session::fs_watch::FsWatchCapabilities::none(),
    )).await;
}

pub(super) async fn persist_peer(receiver: &mut ReceiverFixture) -> (Arc<SessionActor>, crate::agent::subagent::PromptTurnReceipt, xai_chat_state::ParentTask) {
    let tx = receiver.actor.notifications.persistence_tx.clone();
    let (gateway, gateway_rx) = mpsc::unbounded_channel();
    super::super::super::support::drain_gateway(gateway_rx);
    let (mut peer, _) = super::super::super::support::create_test_actor_with_chat_persistence(
        0, 256_000, 85, gateway, tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(tx.clone())),
    ).await;
    peer.notifications.persistence_tx = tx;
    let peer = Arc::new(peer);
    tests::start_turn(&peer, "held-tool-turn").await;
    let expected_task = xai_chat_state::ParentTask::Instance {
        id: peer.state.lock().await.running_task.as_ref().unwrap().identity.as_ref().clone(),
    };
    let (sink, mut receipts) = mpsc::channel(1);
    let (reply, response) = oneshot::channel();
    let (completion, _completed) = mpsc::unbounded_channel();
    let admitting = tokio::task::spawn_local({
        let peer = peer.clone();
        async move { peer.admit_parent_agent_message_for_test(
            ActiveAgentMessage { message_id: "reused-prompt-peer".into(), sender_session_id: "owning-parent".into(), text: Arc::from("another live instance owns this input") },
            ActiveAgentMessageSource::Agent, ActiveAgentMessageOperation::Steer, sink, reply, completion,
        ).await; }
    });
    let (admission, proof, ack) = bounded(receiver.acknowledgements.recv()).await.unwrap();
    assert_eq!(admission.effective, ParentMessageOperation::Steer { prompt_id: "held-tool-turn".into(), parent_task: expected_task.clone() });
    ack.send(proof).unwrap();
    bounded(admitting).await.unwrap();
    assert_eq!(response.await.unwrap(), ActiveMessageAdmission::Admitted);
    let receipt = receipts.recv().await.unwrap();
    assert_eq!(receipt.prompt_id, "parent-message-reused-prompt-peer");
    (peer, receipt, expected_task)
}

#[tokio::test(flavor = "current_thread")]
async fn hard_shutdown_durably_refuses_unprojected_parent_without_cancelling_reused_prompt_peer() {
    tokio::task::LocalSet::new().run_until(async {
        for operation in [ActiveAgentMessageOperation::Steer, ActiveAgentMessageOperation::Interject] {
            let directory = tempfile::tempdir().unwrap();
            let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
            let mut receiver = receiver_with_cuts(directory, storage, ReceiverBarrier::HoldNext).await;
            // Direct-local admission exercises the same maintained receiver and durable owner for both operations.
            let (sink, mut receipts) = mpsc::channel(1);
            let (reply, response) = oneshot::channel();
            let (completion, _completed) = mpsc::unbounded_channel();
            let original_task = receiver.actor.state.lock().await.running_task.as_ref().unwrap().identity.as_ref().clone();
            let admitting = tokio::task::spawn_local({
                let actor = receiver.actor.clone();
                async move { actor.admit_parent_agent_message_for_test(
                    tests::message_with_text("refused-before-safe-point", "literal refused parent input"),
                    ActiveAgentMessageSource::Human, operation, sink, reply, completion,
                ).await; }
            });
            let (admission, proof, ack) = bounded(receiver.acknowledgements.recv()).await.unwrap();
            ack.send(proof).unwrap();
            bounded(admitting).await.unwrap();
            assert_eq!(response.await.unwrap(), ActiveMessageAdmission::Admitted);
            let mut receipt = receipts.recv().await.unwrap();
            assert_eq!(receipt.prompt_id, "parent-message-refused-before-safe-point");
            let expected_task = original_task;
            assert!(matches!(&admission.effective,
                ParentMessageOperation::Steer { parent_task: xai_chat_state::ParentTask::Instance { id }, .. }
                | ParentMessageOperation::Interject { parent_task: xai_chat_state::ParentTask::Instance { id }, .. } if id == &expected_task));
            let (_peer, mut peer_receipt, peer_task) = persist_peer(&mut receiver).await;
            assert_ne!(peer_task, xai_chat_state::ParentTask::Instance { id: expected_task });
            let draining = tokio::task::spawn_local({
                let actor = receiver.actor.clone();
                async move { actor.drain_parent_messages_at_safe_point().await }
            });
            let (flushed, release_barrier) = bounded(receiver.barriers.recv()).await.unwrap();
            assert!(flushed.is_ok());
            assert!(persisted_bindings(&receiver).await.is_empty());
            let lock = JsonlStorageAdapter::lock_append(&receiver._directory.path().join(crate::session::storage::CHAT_HISTORY_FILE)).unwrap();
            let (_, empty) = mpsc::unbounded_channel();
            let mut settlements = std::mem::replace(&mut receiver.settlements, empty);
            let teardown = tokio::task::spawn_local(async move {
                actual_shutdown(&mut receiver, crate::session::ShutdownKind::CancelRunningTurn).await;
                receiver
            });
            bounded(settlements.recv()).await.unwrap();
            assert!(matches!(receipt.result.try_recv(), Err(oneshot::error::TryRecvError::Empty)), "terminal receipt preceded durable settlement");
            drop(lock);
            let mut receiver = bounded(teardown).await.unwrap();
            release_barrier.send(flushed).unwrap();
            assert!(!bounded(draining).await.unwrap());
            assert!(bounded(receipt.result).await.unwrap().is_err());
            assert!(matches!(peer_receipt.result.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            stop_persistence(&mut receiver).await;
            let reopened = JsonlStorageAdapter::with_explicit_session_dir(receiver._directory.path().to_path_buf());
            let (lease, summary) = reopened.claim_receiver(&receiver.actor.session_info).await.unwrap();
            assert!(matches!(summary.completion_delivery.receiver.records[&admission.delivery_id()].execution, DeliveryExecution::Cancelled { .. }));
            let peer_record = &summary.completion_delivery.receiver.records["parent-message/reused-prompt-peer"];
            assert!(matches!(peer_record.execution, DeliveryExecution::Queued { .. }));
            let DeliveryProvenance::ParentMessage { admission: peer_admission } = &peer_record.provenance else { panic!("peer provenance lost"); };
            assert_eq!(peer_admission.effective, ParentMessageOperation::Steer { prompt_id: "held-tool-turn".into(), parent_task: peer_task });
            let loaded = reopened.load_session(&receiver.actor.session_info).await.unwrap();
            assert!(loaded.chat_history.iter().all(|item| item.text_content() != "literal refused parent input"));
            assert!(persisted_bindings(&receiver).await.is_empty());
            drop(lease);
            let (replacement, persistence, _proxy, _projections) = replacement(receiver._directory.path()).await;
            replacement.resume_durable_deliveries(ReceiverResume::Recovery { deliveries: summary.completion_delivery.receiver.recoverable_ids() }).await;
            let state = replacement.state.lock().await;
            assert!(state.pending_inputs.iter().all(|item| item.prompt_id != "parent-message-refused-before-safe-point"));
            let peer = state.pending_inputs.iter().find(|item| item.prompt_id == "parent-message-reused-prompt-peer").unwrap();
            assert_eq!(SessionActor::durable_delivery_payload(&peer.prompt_blocks).unwrap(), "another live instance owns this input");
            drop(state);
            drop(persistence);
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn unavailable_teardown_owner_cannot_publish_terminal_settlement() {
    tokio::task::LocalSet::new().run_until(async {
        let mut receiver = receiver().await;
        let Producer { delivery, reply, outcome, _coordinator, _spawn, .. } = producer().await;
        let message_id = delivery.message().message_id.clone();
        let (admitting, mut receipts) = dispatch(receiver.actor.clone(), delivery, reply);
        let (_, proof, ack) = bounded(receiver.acknowledgements.recv()).await.unwrap();
        ack.send(proof).unwrap();
        bounded(admitting).await.unwrap();
        assert_eq!(bounded(outcome).await.unwrap(), ActiveAgentMessageOutcome::Accepted { message_id: message_id.clone() });
        let mut receipt = receipts.recv().await.unwrap();
        stop_persistence(&mut receiver).await;
        receiver._proxy.abort();
        assert!(bounded(&mut receiver._proxy).await.unwrap_err().is_cancelled());
        assert!(receiver.actor.settle_all_parent_messages(TerminalCause::HardTeardown).await.is_err());
        assert!(matches!(receipt.result.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
        assert!(!receiver.actor.state.lock().await.message_delivery.is_empty());
        let reopened = JsonlStorageAdapter::with_explicit_session_dir(receiver._directory.path().to_path_buf());
        let (lease, summary) = reopened.claim_receiver(&receiver.actor.session_info).await.unwrap();
        assert!(matches!(summary.completion_delivery.receiver.records[&format!("parent-message/{message_id}")].execution, DeliveryExecution::Queued { .. }));
        drop(lease);
        execute_recovered_literal(receiver._directory.path(), &message_id).await;
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn human_receipt_projects_literal_text_without_agent_authorship() {
    tokio::task::LocalSet::new().run_until(async {
        let mut receiver = receiver().await;
        let (sink, mut receipts) = mpsc::channel(1);
        let (reply, response) = oneshot::channel();
        let (completion, _completed) = mpsc::unbounded_channel();
        let admitting = tokio::task::spawn_local({
            let actor = receiver.actor.clone();
            async move { actor.admit_parent_agent_message_for_test(
                ActiveAgentMessage { message_id: "human-literal".into(), sender_session_id: "owning-parent".into(), text: Arc::from("literal human request") },
                ActiveAgentMessageSource::Human, ActiveAgentMessageOperation::Steer, sink, reply, completion,
            ).await; }
        });
        let (admission, proof, ack) = bounded(receiver.acknowledgements.recv()).await.unwrap();
        assert_eq!(admission.sender_session_id, "owning-parent");
        assert_eq!(admission.source, ParentMessageSource::Human);
        assert_eq!(admission.text, "literal human request");
        ack.send(proof).unwrap();
        bounded(admitting).await.unwrap();
        assert_eq!(response.await.unwrap(), ActiveMessageAdmission::Admitted);
        let mut receipt = receipts.recv().await.unwrap();
        assert_eq!(receipt.prompt_id, "parent-message-human-literal");
        assert!(matches!(receipt.result.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
        assert!(receiver.actor.drain_parent_messages_at_safe_point().await);
        let loaded = receiver.storage.load_session(&receiver.actor.session_info).await.unwrap();
        let items: Vec<_> = loaded.chat_history.iter().filter(|item| item.text_content() == "literal human request").cloned().collect();
        let [ConversationItem::User(user)] = items.as_slice() else { panic!("human input was not projected exactly once"); };
        assert_eq!(user.synthetic_reason, xai_grok_sampling_types::SyntheticReason::ParentHumanMessage);
        let prepared = xai_chat_state::compaction_utils::ModelRequestHistory::from_raw(items).into_items();
        let [ConversationItem::User(user)] = prepared.as_slice() else { panic!("human model source changed"); };
        assert!(matches!(user.content.as_slice(), [xai_grok_sampling_types::ContentPart::Text { text }] if text.as_ref() == "literal human request"));
        let bindings = persisted_bindings(&receiver).await;
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].delivery_id, "parent-message/human-literal");
        use sha2::Digest;
        assert_eq!(bindings[0].payload_digest, format!("{:x}", sha2::Sha256::digest(b"literal human request")));
        actual_shutdown(&mut receiver, crate::session::ShutdownKind::Graceful).await;
        assert!(bounded(receipt.result).await.unwrap().is_err());
    }).await;
}

type ProjectionCut = mpsc::UnboundedReceiver<(xai_chat_state::InputProjection, oneshot::Sender<()>)>;

async fn replacement(directory: &std::path::Path) -> (
    Arc<SessionActor>, tokio_util::task::AbortOnDropHandle<()>, tokio_util::task::AbortOnDropHandle<()>, ProjectionCut,
) {
    let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.to_path_buf()));
    let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
    let persistence = crate::session::persistence::durable_update_tests::test_actor(info, storage);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (observed, projections) = mpsc::unbounded_channel();
    let real = persistence.handle.tx.clone();
    let proxy = tokio::task::spawn_local(async move {
        while let Some(message) = rx.recv().await {
            match message {
                PersistenceMsg::AppendInputAndAck { projection: projection @ xai_chat_state::InputProjection::Delivery(_), admission, respond_to } => {
                    let (reply, response) = oneshot::channel();
                    real.send(PersistenceMsg::AppendInputAndAck { projection: projection.clone(), admission, respond_to: reply }).unwrap();
                    let result = response.await.unwrap();
                    assert!(result.is_ok());
                    let (release, receive) = oneshot::channel();
                    observed.send((projection, release)).unwrap();
                    let _ = receive.await;
                    let _ = respond_to.send(result);
                }
                other => real.send(other).unwrap(),
            }
        }
    });
    let (gateway, gateway_rx) = mpsc::unbounded_channel();
    super::super::super::support::drain_gateway(gateway_rx);
    let (mut actor, _) = super::super::super::support::create_test_actor_with_chat_persistence(
        0, 256_000, 85, gateway, tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(tx.clone())),
    ).await;
    actor.notifications.persistence_tx = tx;
    let actor = Arc::new(actor);
    tests::start_turn(&actor, "replacement-anchor").await;
    (actor, tokio_util::task::AbortOnDropHandle::new(persistence.task), tokio_util::task::AbortOnDropHandle::new(proxy), projections)
}

pub(super) fn assert_agent_model_projection(item: &ConversationItem) {
    let prepared = xai_chat_state::compaction_utils::ModelRequestHistory::from_raw(vec![item.clone()]).into_items();
    let [ConversationItem::User(user)] = prepared.as_slice() else { panic!("wrong model input source"); };
    assert!(matches!(user.content.as_slice(),
        [xai_grok_sampling_types::ContentPart::Text { text: label }, xai_grok_sampling_types::ContentPart::Text { text }]
        if label.as_ref() == "[Message authored by another agent; not a human request or approval.]"
            && text.as_ref() == "claim-bound text stays behind the tool boundary"));
}

pub(super) async fn execute_recovered_literal(directory: &std::path::Path, message_id: &str) {
    let (actor, _persistence, _proxy, mut projections) = replacement(directory).await;
    actor.resume_durable_deliveries(ReceiverResume::Recovery { deliveries: vec![format!("parent-message/{message_id}")] }).await;
    {
        let mut state = actor.state.lock().await;
        let recovered = state.pending_inputs.iter().find(|item| item.prompt_id == format!("parent-message-{message_id}")).unwrap();
        assert_eq!(SessionActor::durable_delivery_payload(&recovered.prompt_blocks).unwrap(), "claim-bound text stays behind the tool boundary");
        assert!(matches!(recovered.input_origin.as_prompt_origin(), PromptOrigin::ParentAgentMessage { sender_session_id, message_id: id } if sender_session_id == "owning-parent" && id == message_id));
        state.running_task.take().unwrap().handle.abort();
        state.pending_inputs.retain(|item| item.prompt_id != "replacement-anchor");
    }
    let (completion, _completed) = mpsc::unbounded_channel();
    SessionActor::maybe_start_running_task(actor.clone(), completion).await;
    let (projection, release) = bounded(projections.recv()).await.unwrap();
    let xai_chat_state::InputProjection::Delivery(projection) = projection else { panic!("not delivery execution"); };
    assert_eq!(projection.binding.delivery_id, format!("parent-message/{message_id}"));
    assert_eq!(projection.binding.turn_id, format!("parent-message-{message_id}"));
    assert_eq!(projection.item.text_content(), "claim-bound text stays behind the tool boundary");
    assert_agent_model_projection(&projection.item);
    let ConversationItem::User(user) = &projection.item else { panic!("not source-specific input"); };
    assert_eq!(user.synthetic_reason, xai_grok_sampling_types::SyntheticReason::AgentMessage);
    use sha2::Digest;
    assert_eq!(projection.binding.payload_digest, format!("{:x}", sha2::Sha256::digest(b"claim-bound text stays behind the tool boundary")));
    let task = actor.state.lock().await.running_task.take().unwrap();
    let recovered_task = xai_chat_state::ParentTask::Instance { id: task.identity.as_ref().clone() };
    assert_eq!(projection.binding.parent_task, recovered_task);
    task.handle.abort();
    drop(release);
    let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.to_path_buf());
    let loaded = reopened.load_session(&actor.session_info).await.unwrap();
    assert_eq!(loaded.chat_history.iter().filter(|item| item.text_content() == "claim-bound text stays behind the tool boundary").count(), 1);
    let bytes = std::fs::read_to_string(directory.join(crate::session::storage::CHAT_HISTORY_FILE)).unwrap();
    let persisted: Vec<xai_chat_state::DeliveryProjection> = bytes.lines().filter_map(|line| {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        if value.get("_session_delivery").is_some() { Some(serde_json::from_value(value).unwrap()) } else { None }
    }).collect();
    let [persisted] = persisted.as_slice() else { panic!("recovery must persist one delivery binding"); };
    assert_eq!(persisted.binding.delivery_id, format!("parent-message/{message_id}"));
    assert_eq!(persisted.binding.turn_id, format!("parent-message-{message_id}"));
    assert_eq!(persisted.binding.parent_task, recovered_task);
    assert_eq!(persisted.binding.payload_digest, format!("{:x}", sha2::Sha256::digest(b"claim-bound text stays behind the tool boundary")));
    assert_eq!(persisted.item.text_content(), "claim-bound text stays behind the tool boundary");
    let ConversationItem::User(user) = &persisted.item else { panic!("serialized recovery source changed"); };
    assert_eq!(user.synthetic_reason, xai_grok_sampling_types::SyntheticReason::AgentMessage);
    assert_agent_model_projection(&persisted.item);
    let reopened_item = loaded.chat_history.iter().find(|item| item.text_content() == "claim-bound text stays behind the tool boundary").unwrap();
    assert_agent_model_projection(reopened_item);
}
