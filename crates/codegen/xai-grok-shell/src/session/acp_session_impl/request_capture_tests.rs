use super::*;
use crate::session::storage::request_capture::{SnapshotRead, SnapshotReadResult};
use crate::session::storage::{StorageAdapter, jsonl::JsonlStorageAdapter};
use xai_grok_sampler::request_capture::LocalRequestEvidence;

async fn persistence(actor: &SessionActor, directory: &std::path::Path) -> PersistenceHandle {
    crate::session::persistence::new_with_explicit_dir(
        &actor.session_info, directory.to_owned(), acp::ModelId::new("test"),
        super::super::updates_tests::test_persistence_sampling_client(), "test".into(),
        crate::session::persistence::ExplicitSessionOpen::New { identity: None, next_trace_turn: None, parent_session_id: None },
    ).await.unwrap()
}

#[tokio::test]
async fn input_binding_uses_session_prompt_and_actual_append_without_extra_model_input() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut notifications) = tokio::sync::mpsc::unbounded_channel();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let actor = super::super::support::create_test_actor(0, 256_000, 85, gateway, tx).await;
        let first = actor.chat_state_handle.push_user_message_and_ack(ConversationItem::user("duplicate")).await.unwrap();
        let second = actor.chat_state_handle.push_user_message_and_ack(ConversationItem::user("duplicate")).await.unwrap();
        let before = serde_json::to_value(actor.chat_state_handle.get_conversation().await).unwrap();
        for (prompt_id, applied) in [("first-prompt", first), ("second-prompt", second)] {
            actor.current_turn_number.set(7);
            let input = actor.prompt_input_ref(prompt_id);
            actor.current_turn_number.set(99);
            actor.observe_input_source(input, Ok(applied));
            let message = notifications.recv().await.unwrap();
            let xai_acp_lib::AcpClientMessage::ExtNotification(args) = message else { panic!("input binding notification"); };
            assert_eq!(args.request.method.as_ref(), INPUT_SOURCE_NOTIFICATION);
            let wire: serde_json::Value = serde_json::from_str(args.request.params.get()).unwrap();
            assert_eq!(wire["input"]["promptId"], prompt_id);
            assert_eq!(wire["input"]["sessionId"], actor.session_info.id.to_string());
            assert_eq!(wire["input"]["turn"], 7);
            assert_eq!(wire["_meta"]["requestSource"]["source"], serde_json::to_value(applied.source).unwrap());
            let notification: InputSourceNotification = serde_json::from_value(wire).unwrap();
            assert!(!format!("{notification:?}").contains("duplicate"));
            let InputSourceBinding::Applied { meta } = notification.binding else { panic!("actual applied binding"); };
            assert!(matches!(meta.publication.durability, SourceDurability::UnconfirmedIndependentHistoryChannel));
        }
        assert_ne!(first.source, second.source);
        actor.observe_input_source(actor.prompt_input_ref("legacy-receipt"), Err(InputSourceUnknown::StrictReceiptSourceUnavailable));
        let message = notifications.recv().await.unwrap();
        let xai_acp_lib::AcpClientMessage::ExtNotification(args) = message else { panic!("unknown binding"); };
        let wire: serde_json::Value = serde_json::from_str(args.request.params.get()).unwrap();
        assert_eq!(wire["binding"], "unknown");
        assert!(wire.get("_meta").is_none());
        assert_eq!(serde_json::to_value(actor.chat_state_handle.get_conversation().await).unwrap(), before);
    }).await;
}

#[tokio::test]
async fn actual_capture_fifo_keeps_submit_origin_after_cancel_without_claiming_later_history_rewrite() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut notifications) = tokio::sync::mpsc::unbounded_channel();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut actor = super::super::support::create_test_actor(0, 256_000, 85, gateway, tx).await;
        let directory = tempfile::tempdir().unwrap();
        let persistent = persistence(&actor, directory.path()).await;
        actor.notifications.persistence_tx = persistent.tx.clone();
        actor.current_turn_number.set(7);
        let (id, events, wire) = crate::session::storage::request_capture::tests::captured("actual shell capture input").await;
        let previous = vec![ConversationItem::user("earlier canonical admission")];
        let storage = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned());
        storage.replace_chat_history(&actor.session_info, &previous).await.unwrap();
        let original = std::fs::read(directory.path().join(crate::session::storage::CHAT_HISTORY_FILE)).unwrap();
        actor.begin_request_capture(id, &ConversationRequest::default());
        actor.current_turn_number.set(99);
        let actor = Arc::new(actor);
        assert!(actor.turn_stream_drained.lock().is_empty());
        for (attempt, evidence) in events {
            actor.handle_sampling_event(xai_grok_sampler::SamplingEvent::LocalRequest {
                request_id: xai_grok_sampler::RequestId::from(id.0.to_string()), attempt, evidence,
            }).await;
        }
        assert_eq!(std::fs::read(directory.path().join(crate::session::storage::CHAT_HISTORY_FILE)).unwrap(), original);
        let mut observed = Vec::new();
        while let Ok(message) = notifications.try_recv() {
            if let xai_acp_lib::AcpClientMessage::ExtNotification(args) = message {
                if args.request.method.as_ref() == REQUEST_SNAPSHOT_NOTIFICATION {
                    observed.push(serde_json::from_str::<RequestSnapshotNotification>(args.request.params.get()).unwrap());
                }
            }
        }
        assert!(!observed.is_empty());
        assert!(observed.iter().all(|notification| matches!(notification.receipt, CaptureReceipt::Confirmed { .. })
            && notification.receipt.origin().unwrap().turn == 7));
        assert!(observed.iter().any(|notification| matches!(&notification.evidence.observation, RequestObservation::Prepared {
            source_durability: SourceDurability::UnconfirmedIndependentHistoryChannel, body: PreparedBody { byte_len, availability: BodyAvailability::Captured { .. }, .. }, ..
        } if *byte_len == wire.len() as u64)));
        let newer = vec![ConversationItem::user("newer canonical admission from the other channel")];
        persistent.tx.send(PersistenceMsg::ReplaceChatHistory(newer.clone())).unwrap();
        let (respond_to, response) = tokio::sync::oneshot::channel();
        persistent.tx.send(PersistenceMsg::FlushAndAck { respond_to }).unwrap();
        response.await.unwrap().unwrap();
        let expected = newer.into_iter().map(|item| serde_json::to_value(item).unwrap()).collect::<Vec<_>>();
        let actual = std::fs::read_to_string(directory.path().join(crate::session::storage::CHAT_HISTORY_FILE)).unwrap()
            .lines().map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()).collect::<Vec<_>>();
        assert_eq!(actual, expected, "capture cannot overwrite newer canonical admissions with a policy-pruned request");
        let (respond_to, response) = tokio::sync::oneshot::channel();
        persistent.tx.send(PersistenceMsg::ReadRequestSnapshots { request: SnapshotRead::Get { request: id }, respond_to }).unwrap();
        let SnapshotReadResult::Snapshot { snapshot } = response.await.unwrap() else { panic!("passive persistence lookup"); };
        assert_eq!(snapshot.origin.turn, 7);
        assert!(snapshot.observations.iter().any(|entry| matches!(&entry.observation, RequestObservation::Prepared {
            source_durability: SourceDurability::UnconfirmedIndependentHistoryChannel, ..
        })), "a later FIFO update/history ack cannot retroactively prove the independent migration channel");
    }).await;
}

#[tokio::test]
async fn capture_only_storage_failure_does_not_poison_primary_persistence() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut notifications) = tokio::sync::mpsc::unbounded_channel();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut actor = super::super::support::create_test_actor(0, 256_000, 85, gateway, tx).await;
        let directory = tempfile::tempdir().unwrap();
        let persistent = persistence(&actor, directory.path()).await;
        actor.notifications.persistence_tx = persistent.tx.clone();
        std::fs::write(directory.path().join(crate::session::storage::request_capture::DIRECTORY), b"unavailable capture directory").unwrap();
        let id = RequestSnapshotId(uuid::Uuid::new_v4());
        actor.begin_request_capture(id, &ConversationRequest::default());
        let actor = Arc::new(actor);
        actor.handle_sampling_event(xai_grok_sampler::SamplingEvent::LocalRequest {
            request_id: xai_grok_sampler::RequestId::from(id.0.to_string()),
            attempt: xai_grok_sampler::request_capture::RequestAttemptId(1), evidence: LocalRequestEvidence::DispatchStarted,
        }).await;
        let message = notifications.recv().await.unwrap();
        let xai_acp_lib::AcpClientMessage::ExtNotification(args) = message else { panic!("capture notification"); };
        let notification: RequestSnapshotNotification = serde_json::from_str(args.request.params.get()).unwrap();
        assert!(matches!(notification.receipt, CaptureReceipt::Unconfirmed { .. }));
        persistent.tx.send(PersistenceMsg::Chat(ConversationItem::user("primary request continues"))).unwrap();
        let (respond_to, response) = tokio::sync::oneshot::channel();
        persistent.tx.send(PersistenceMsg::FlushAndAck { respond_to }).unwrap();
        response.await.unwrap().unwrap();
        let history = std::fs::read_to_string(directory.path().join(crate::session::storage::CHAT_HISTORY_FILE)).unwrap();
        assert!(history.contains("primary request continues"));
        assert!(!actor.notifications.is_disk_full());
    }).await;
}
