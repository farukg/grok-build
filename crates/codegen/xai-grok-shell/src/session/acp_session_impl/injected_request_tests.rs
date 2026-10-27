use super::super::support::create_test_actor;
use super::*;
use crate::session::storage::{StorageAdapter, SessionUpdate, SessionUpdateEnvelope};
use xai_chat_state::InjectedRequestRecord;

#[tokio::test]
async fn injected_record_is_durable_before_live_delivery_and_replays_losslessly() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::unbounded_channel();
        let (persistence_tx, mut persistence_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut actor = create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
        let root = tempfile::tempdir().unwrap();
        actor.session_info.cwd = root.path().to_string_lossy().into_owned();
        let persistence = crate::session::persistence::new_with_explicit_dir(
            &actor.session_info, root.path().to_path_buf(), acp::ModelId::new("test-model"),
            super::super::updates_tests::test_persistence_sampling_client(), "test-model".into(),
            crate::session::persistence::ExplicitSessionOpen::New {
                identity: None, next_trace_turn: None, parent_session_id: None,
            },
        ).await.unwrap();
        let records: Vec<_> = (0..32).map(|index| InjectedRequestRecord::MemoryReminder {
            source: Default::default(),
            text: format!("{index}:{}\nVISIBLE_TAIL_{index}", "ä".repeat(5000)),
        }).collect();
        let refs: Vec<_> = records.iter().collect();
        let publish = actor.send_injected_request_records(&refs);
        let persist = async {
            let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = persistence_rx.recv().await.unwrap() else {
                panic!("one acknowledged request batch is required");
            };
            assert_eq!(updates.len(), records.len());
            assert!(persistence_rx.try_recv().is_err(), "no per-record persistence operations");
            assert!(gateway_rx.try_recv().is_err(), "live publication must wait for durability");
            let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
            persistence.tx.send(PersistenceMsg::AppendUpdatesAndAck { updates, respond_to: ack_tx }).unwrap();
            let result = ack_rx.await.unwrap();
            assert!(result.is_ok());
            assert!(gateway_rx.try_recv().is_err(), "withholding the batch ack must withhold every live record");
            respond_to.send(result).unwrap();
        };
        let (result, ()) = tokio::join!(publish, persist);
        result.unwrap();
        let mut delivered = Vec::new();
        while let Ok(message) = gateway_rx.try_recv() {
            let xai_acp_lib::AcpClientMessage::ExtNotification(args) = message else {
                panic!("request records use individual extension notifications");
            };
            assert_eq!(args.request.method.as_ref(), "x.ai/session_notification");
            delivered.push(serde_json::from_str::<serde_json::Value>(args.request.params.get()).unwrap());
        }
        assert_eq!(delivered.len(), records.len());
        let reopened = crate::session::storage::jsonl::JsonlStorageAdapter::with_explicit_session_dir(root.path().to_path_buf());
        let path = reopened.updates_file_path(&actor.session_info).unwrap();
        let lines = std::fs::read_to_string(path).unwrap();
        let mut replayed = Vec::new();
        let mut persisted_notifications = Vec::new();
        let mut event_ids = std::collections::HashSet::new();
        for line in lines.lines() {
            let SessionUpdate::Xai(notification) = SessionUpdateEnvelope::from_str(line).unwrap() else { continue; };
            if let XaiSessionUpdate::InjectedContextRecord { kind, text } = &notification.update {
                persisted_notifications.push(serde_json::to_value(&*notification).unwrap());
                assert_eq!(kind, "memory_reminder");
                replayed.push(text.clone());
                assert!(event_ids.insert(notification.meta.as_ref().unwrap()["eventId"].as_str().unwrap().to_owned()));
            }
        }
        assert_eq!(replayed, records.iter().map(|record| record.text().to_owned()).collect::<Vec<_>>());
        assert_eq!(delivered, persisted_notifications, "batch acknowledgement must not restamp or reorder individual live records");
    }).await;
}

#[tokio::test]
async fn sampling_request_boundary_publishes_changed_second_view_but_not_identical_injections() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::unbounded_channel();
        let (persistence_tx, mut persistence_rx) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
        for (iteration, addition) in [Some("first injected body"), Some("second injected body"), None].into_iter().enumerate() {
            if let Some(body) = addition {
                actor.chat_state_handle.push_user_message(ConversationItem::system_reminder(body));
            }
            let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), format!("r-{iteration}")).await.unwrap();
            let actual_items = built.request.items.clone();
            let prepare = actor.prepare_request_for_sampling(built, None);
            if let Some(expected_body) = addition {
                let acknowledge = async {
                    let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = persistence_rx.recv().await.unwrap() else {
                        panic!("one changed request-view batch");
                    };
                    assert_eq!(updates.len(), 1);
                    let SessionUpdate::Xai(notification) = &updates[0] else { panic!("request record"); };
                    assert!(matches!(&notification.update, XaiSessionUpdate::InjectedContextRecord { kind, text }
                        if kind == "system_reminder" && text == expected_body));
                    assert!(gateway_rx.try_recv().is_err());
                    respond_to.send(Ok(())).unwrap();
                };
                let (request, ()) = tokio::join!(prepare, acknowledge);
                assert_eq!(serde_json::to_value(&request.items).unwrap(), serde_json::to_value(&actual_items).unwrap());
                assert!(request.items.iter().any(|item| item.text_content() == expected_body));
                assert!(gateway_rx.try_recv().is_ok());
                assert!(gateway_rx.try_recv().is_err());
            } else {
                let request = prepare.await;
                assert_eq!(serde_json::to_value(&request.items).unwrap(), serde_json::to_value(&actual_items).unwrap());
                assert!(persistence_rx.try_recv().is_err());
                assert!(gateway_rx.try_recv().is_err());
            }
        }
    }).await;
}

#[tokio::test]
async fn equal_body_actor_injections_publish_each_new_occurrence_once() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::unbounded_channel();
            let (persistence_tx, mut persistence_rx) = tokio::sync::mpsc::unbounded_channel();
            let actor = create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
            let body = "equal injected occurrence";
            let mut event_ids = std::collections::HashSet::new();
            for iteration in 0..3 {
                if iteration < 2 {
                    actor.chat_state_handle.push_user_message(ConversationItem::system_reminder(body));
                }
                let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), format!("equal-{iteration}")).await.unwrap();
                let expected_count = (iteration + 1).min(2);
                assert_eq!(built.items.iter().filter(|item| item.text_content() == body).count(), expected_count);
                assert_eq!(built.injected_records.iter().filter(|record| record.text() == body).count(), expected_count);
                let prepare = actor.prepare_request_for_sampling(built, None);
                let request = if iteration < 2 {
                    let acknowledge = async {
                        let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = persistence_rx.recv().await.unwrap() else {
                            panic!("each new occurrence requires one batch ACK");
                        };
                        assert_eq!(updates.len(), 1);
                        assert!(persistence_rx.try_recv().is_err());
                        assert!(gateway_rx.try_recv().is_err(), "the occurrence must wait for its batch ACK");
                        let SessionUpdate::Xai(notification) = &updates[0] else { panic!("injected record"); };
                        assert!(matches!(&notification.update, XaiSessionUpdate::InjectedContextRecord { kind, text }
                            if kind == "system_reminder" && text == body));
                        let persisted = serde_json::to_value(&**notification).unwrap();
                        respond_to.send(Ok(())).unwrap();
                        persisted
                    };
                    let (request, persisted) = tokio::join!(prepare, acknowledge);
                    let xai_acp_lib::AcpClientMessage::ExtNotification(args) = gateway_rx.try_recv().unwrap() else {
                        panic!("individual live record");
                    };
                    let live: serde_json::Value = serde_json::from_str(args.request.params.get()).unwrap();
                    assert_eq!(live, persisted);
                    assert!(event_ids.insert(live["_meta"]["eventId"].as_str().unwrap().to_owned()));
                    request
                } else {
                    prepare.await
                };
                assert_eq!(request.items.iter().filter(|item| item.text_content() == body).count(), expected_count);
                assert!(persistence_rx.try_recv().is_err(), "the unchanged view has no batch");
                assert!(gateway_rx.try_recv().is_err(), "only the new occurrence is published");
            }
            assert_eq!(event_ids.len(), 2);
        }).await.expect("equal-body occurrences must not disappear into set membership");
    }).await;
}

#[tokio::test]
async fn failed_request_record_persistence_does_not_publish_live_content() {
    tokio::task::LocalSet::new().run_until(async {
        for lose_ack in [false, true] {
            let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::unbounded_channel();
            let (persistence_tx, mut persistence_rx) = tokio::sync::mpsc::unbounded_channel();
            let actor = create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
            actor.chat_state_handle.push_user_message(ConversationItem::system_reminder("changed view"));
            let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "r".into()).await.unwrap();
            let publish = actor.prepare_request_for_sampling(built, None);
            let reject = async {
                let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = persistence_rx.recv().await.unwrap() else {
                    panic!("expected acknowledged request batch");
                };
                assert_eq!(updates.len(), 1);
                assert!(gateway_rx.try_recv().is_err());
                if lose_ack { drop(respond_to); } else {
                    respond_to.send(Err(crate::session::request_capture::PublicationFailure {
                        cause: crate::session::request_capture::CaptureFailure::io(&std::io::Error::new(std::io::ErrorKind::StorageFull, "full")),
                        progress: crate::session::request_capture::PublicationProgress::NotCommitted,
                    })).unwrap();
                }
            };
            let (result, ()) = tokio::join!(publish, reject);
            assert!(result.items.iter().any(|item| item.text_content() == "changed view"), "optional publication cannot stop or prune model work");
            let xai_acp_lib::AcpClientMessage::ExtNotification(args) = gateway_rx.try_recv().unwrap() else { panic!("typed capture diagnostic"); };
            assert_eq!(args.request.method.as_ref(), "x.ai/session/request_sources/unconfirmed");
            let failure: crate::session::request_capture::SourcePublicationFailure = serde_json::from_str(args.request.params.get()).unwrap();
            assert_eq!(failure.sources.len(), 1);
            if lose_ack { assert!(matches!(failure.cause, crate::session::request_capture::CaptureFailure::ReplyLost)); }
            let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "retry".into()).await.unwrap();
            let retry = actor.prepare_request_for_sampling(built, None);
            let acknowledge = async {
                if lose_ack {
                    let PersistenceMsg::ReadPublishedRequestSources { respond_to } = persistence_rx.recv().await.unwrap() else { panic!("lost reply requires journal reconciliation"); };
                    respond_to.send(Ok(Vec::new())).unwrap();
                }
                let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = persistence_rx.recv().await.unwrap() else { panic!("definitely unapplied source must be retried"); };
                assert_eq!(updates.len(), 1);
                let SessionUpdate::Xai(notification) = &updates[0] else { panic!("source row"); };
                let publication: crate::session::request_capture::SourcePublication = serde_json::from_value(notification.meta.as_ref().unwrap()[crate::session::request_capture::SOURCE_PUBLICATION_META].clone()).unwrap();
                assert_eq!(publication.source, failure.sources[0], "retry the original occurrence, not a new admission");
                respond_to.send(Ok(())).unwrap();
            };
            let (retry, ()) = tokio::join!(retry, acknowledge);
            assert_eq!(retry.items.iter().filter(|item| item.text_content() == "changed view").count(), 1);
            assert!(persistence_rx.try_recv().is_err());
            let xai_acp_lib::AcpClientMessage::ExtNotification(args) = gateway_rx.try_recv().unwrap() else { panic!("retried source publication"); };
            assert_eq!(args.request.method.as_ref(), "x.ai/session_notification");
            assert!(gateway_rx.try_recv().is_err());
        }
    }).await;
}

#[tokio::test]
async fn a_new_user_turn_does_not_republish_historical_injections() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::unbounded_channel();
        let (persistence_tx, mut persistence_rx) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
        actor.push_system_reminder("once across user turns");
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "first".into()).await.unwrap();
        let original = serde_json::to_value(&built.request.items).unwrap();
        let publish = actor.prepare_request_for_sampling(built, None);
        let acknowledge = async {
            let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = persistence_rx.recv().await.unwrap() else { panic!("source publication"); };
            assert_eq!(updates.len(), 1);
            let SessionUpdate::Xai(notification) = &updates[0] else { panic!("source row"); };
            let metadata: crate::session::request_capture::SourcePublication = serde_json::from_value(
                notification.meta.as_ref().unwrap()[crate::session::request_capture::SOURCE_PUBLICATION_META].clone(),
            ).unwrap();
            assert!(matches!(metadata.source, xai_grok_sampling_types::SourceOccurrence::Admitted { .. }));
            assert!(matches!(metadata.durability, crate::session::request_capture::SourceDurability::UnconfirmedIndependentHistoryChannel));
            respond_to.send(Ok(())).unwrap();
        };
        let (first, ()) = tokio::join!(publish, acknowledge);
        assert_eq!(serde_json::to_value(&first.items).unwrap(), original);
        assert!(gateway_rx.try_recv().is_ok());
        actor.chat_state_handle.push_user_message(ConversationItem::user("second human turn"));
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "second".into()).await.unwrap();
        let original = serde_json::to_value(&built.request.items).unwrap();
        let second = actor.prepare_request_for_sampling(built, None).await;
        assert_eq!(serde_json::to_value(&second.items).unwrap(), original);
        assert!(second.items.iter().any(|item| item.text_content() == "second human turn"));
        assert!(persistence_rx.try_recv().is_err());
        assert!(gateway_rx.try_recv().is_err());
    }).await;
}

#[tokio::test]
async fn restore_and_legacy_do_not_guess_old_ui_identity_or_publish_diagnostic_views() {
    use xai_grok_sampling_types::{SourceOccurrence, SourceOccurrenceId};
    tokio::task::LocalSet::new().run_until(async {
        let (gateway_tx, mut gateway_rx) = tokio::sync::mpsc::unbounded_channel();
        let (persistence_tx, mut persistence_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut actor = create_test_actor(0, 256_000, 85, gateway_tx, persistence_tx).await;
        let directory = tempfile::tempdir().unwrap();
        let persistence = crate::session::persistence::new_with_explicit_dir(
            &actor.session_info, directory.path().to_owned(), acp::ModelId::new("test"),
            super::super::updates_tests::test_persistence_sampling_client(), "test".into(),
            crate::session::persistence::ExplicitSessionOpen::New { identity: None, next_trace_turn: None, parent_session_id: None },
        ).await.unwrap();
        actor.notifications.persistence_tx = persistence.tx.clone();
        let published = InjectedRequestRecord::MemoryReminder {
            source: SourceOccurrence::Admitted { id: SourceOccurrenceId([71; 16]) },
            text: "restored diagnostic occurrence".into(),
        };
        actor.send_injected_request_records(&[&published]).await.unwrap();
        assert!(gateway_rx.try_recv().is_ok());
        let mut retained = ConversationItem::system_reminder("restored diagnostic occurrence");
        *retained.source_mut() = SourceOccurrence::Admitted { id: SourceOccurrenceId([71; 16]) };
        let history = vec![retained, ConversationItem::system_reminder("legacy diagnostic occurrence"), ConversationItem::user("retained human input")];
        let restored: Vec<ConversationItem> = serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
        *actor.notifications.source_publications.lock() = crate::session::request_capture::SourcePublications::restored();
        let mut snapshot = actor.chat_state_handle.snapshot().await.unwrap();
        snapshot.conversation = restored;
        actor.chat_state_handle.restore_snapshot(snapshot);
        let built = actor.chat_state_handle.build_request(vec![], Some("request-only memory".into()), false, None, "c".into(), "restored".into()).await.unwrap();
        let original = serde_json::to_value(&built.request.items).unwrap();
        let request = actor.prepare_request_for_sampling(built, None).await;
        assert_eq!(serde_json::to_value(&request.items).unwrap(), original);
        assert!(request.items.iter().any(|item| item.text_content() == "retained human input"));
        assert!(persistence_rx.try_recv().is_err(), "journal-confirmed restored sources and identity-free legacy/request-only records produce no new publication");
        assert!(gateway_rx.try_recv().is_err());
    }).await;
}

#[tokio::test]
async fn durable_source_claims_survive_compaction_restart_and_old_checkpoint_reintroduction() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut live) = tokio::sync::mpsc::unbounded_channel();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut actor = create_test_actor(0, 256_000, 85, gateway, tx).await;
        let directory = tempfile::tempdir().unwrap();
        let persistence = crate::session::persistence::new_with_explicit_dir(
            &actor.session_info, directory.path().to_owned(), acp::ModelId::new("test"),
            super::super::updates_tests::test_persistence_sampling_client(), "test".into(),
            crate::session::persistence::ExplicitSessionOpen::New { identity: None, next_trace_turn: None, parent_session_id: None },
        ).await.unwrap();
        actor.notifications.persistence_tx = persistence.tx.clone();
        actor.push_system_reminder("same deliberate body across restore");
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "before-restart".into()).await.unwrap();
        actor.prepare_request_for_sampling(built, None).await;
        let first = live.recv().await.unwrap();
        let xai_acp_lib::AcpClientMessage::ExtNotification(first) = first else { panic!("source publication"); };
        let first: XaiSessionNotification = serde_json::from_str(first.request.params.get()).unwrap();
        let source: crate::session::request_capture::SourcePublication = serde_json::from_value(first.meta.unwrap()[crate::session::request_capture::SOURCE_PUBLICATION_META].clone()).unwrap();
        let checkpoint = actor.chat_state_handle.snapshot().await.unwrap();
        *actor.notifications.source_publications.lock() = crate::session::request_capture::SourcePublications::restored();
        actor.chat_state_handle.replace_conversation(Vec::new());
        actor.chat_state_handle.restore_snapshot(checkpoint);
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "after-restart".into()).await.unwrap();
        actor.prepare_request_for_sampling(built, None).await;
        assert!(live.try_recv().is_err(), "a removed source restored from an old checkpoint is not a new occurrence");
        actor.push_system_reminder("same deliberate body across restore");
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "new-occurrence".into()).await.unwrap();
        actor.prepare_request_for_sampling(built, None).await;
        let xai_acp_lib::AcpClientMessage::ExtNotification(second) = live.recv().await.unwrap() else { panic!("new occurrence"); };
        let second: XaiSessionNotification = serde_json::from_str(second.request.params.get()).unwrap();
        let second: crate::session::request_capture::SourcePublication = serde_json::from_value(second.meta.unwrap()[crate::session::request_capture::SOURCE_PUBLICATION_META].clone()).unwrap();
        assert_ne!(source.source, second.source);
        let ids = crate::session::storage::request_capture::published_sources(directory.path()).unwrap();
        assert_eq!(ids, vec![source.source.id().unwrap(), second.source.id().unwrap()]);
        let rows = std::fs::read_to_string(directory.path().join(crate::session::storage::UPDATES_FILE)).unwrap()
            .lines().map(|line| SessionUpdateEnvelope::from_str(line).unwrap()).filter(|update| matches!(update,
                SessionUpdate::Xai(notification) if matches!(&notification.update, XaiSessionUpdate::InjectedContextRecord { .. })
            )).count();
        assert_eq!(rows, 2);
        let claims = directory.path().join(crate::session::storage::request_capture::DIRECTORY).join("source-publications.jsonl");
        std::fs::remove_file(&claims).unwrap();
        *actor.notifications.source_publications.lock() = crate::session::request_capture::SourcePublications::restored();
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "replay-metadata-restore".into()).await.unwrap();
        actor.prepare_request_for_sampling(built, None).await;
        assert!(live.try_recv().is_err(), "exact replay source metadata reconstructs the missing private index without text/index backfill");
        assert!(!claims.exists(), "a passive restore read cannot rewrite history or create an index");
        actor.push_system_reminder("same deliberate body across restore");
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "after-index-migration".into()).await.unwrap();
        actor.prepare_request_for_sampling(built, None).await;
        assert!(live.try_recv().is_ok());
        let ids = crate::session::storage::request_capture::published_sources(directory.path()).unwrap();
        assert_eq!(ids.len(), 3, "the new index must retain replayed claims as well as the new occurrence");
        assert_eq!(&ids[..2], &[source.source.id().unwrap(), second.source.id().unwrap()]);
    }).await;
}

#[tokio::test]
async fn transient_publication_restore_retries_on_next_operation_without_repeating_warning() {
    use crate::session::request_capture::SourcePublications;
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut live) = tokio::sync::mpsc::unbounded_channel();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway, tx).await;
        actor.push_system_reminder("recoverable publication");
        *actor.notifications.source_publications.lock() = SourcePublications::restored();
        for operation in 0..3 {
            let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), format!("recovery-{operation}")).await.unwrap();
            let prepare = actor.prepare_request_for_sampling(built, None);
            let store = async {
                let PersistenceMsg::ReadPublishedRequestSources { respond_to } = rx.recv().await.unwrap() else { panic!("one event-driven recovery read"); };
                if operation < 2 {
                    respond_to.send(Err(std::io::Error::from(std::io::ErrorKind::Interrupted))).unwrap();
                } else {
                    respond_to.send(Ok(Vec::new())).unwrap();
                    let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = rx.recv().await.unwrap() else { panic!("recovery publishes still-unapplied source"); };
                    assert_eq!(updates.len(), 1);
                    respond_to.send(Ok(())).unwrap();
                }
            };
            let (request, ()) = tokio::join!(prepare, store);
            assert_eq!(request.items.iter().filter(|item| item.text_content() == "recoverable publication").count(), 1);
            match operation {
                0 => {
                    let xai_acp_lib::AcpClientMessage::ExtNotification(args) = live.try_recv().unwrap() else { panic!("recovery warning"); };
                    assert_eq!(args.request.method.as_ref(), "x.ai/session/request_sources/unconfirmed");
                }
                1 => assert!(live.try_recv().is_err(), "unchanged unavailable evidence must not spam warnings"),
                2 => {
                    let xai_acp_lib::AcpClientMessage::ExtNotification(args) = live.try_recv().unwrap() else { panic!("recovered publication"); };
                    assert_eq!(args.request.method.as_ref(), "x.ai/session_notification");
                }
                _ => unreachable!(),
            }
            assert!(rx.try_recv().is_err(), "no timer, polling or background restore requests");
        }
    }).await;
}

#[tokio::test]
async fn lost_publication_reply_reconciles_actual_journal_without_republishing_or_model_duplication() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut live) = tokio::sync::mpsc::unbounded_channel();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway, tx).await;
        let directory = tempfile::tempdir().unwrap();
        let persistence = crate::session::persistence::new_with_explicit_dir(
            &actor.session_info, directory.path().to_owned(), acp::ModelId::new("test"),
            super::super::updates_tests::test_persistence_sampling_client(), "test".into(),
            crate::session::persistence::ExplicitSessionOpen::New { identity: None, next_trace_turn: None, parent_session_id: None },
        ).await.unwrap();
        actor.push_system_reminder("committed before reply loss");
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "lost-reply".into()).await.unwrap();
        let publish = actor.prepare_request_for_sampling(built, None);
        let commit_then_lose = async {
            let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = rx.recv().await.unwrap() else { panic!("publication"); };
            let (ack, reply) = tokio::sync::oneshot::channel();
            persistence.tx.send(PersistenceMsg::AppendUpdatesAndAck { updates, respond_to: ack }).unwrap();
            reply.await.unwrap().unwrap();
            drop(respond_to);
        };
        let (request, ()) = tokio::join!(publish, commit_then_lose);
        assert_eq!(request.items.iter().filter(|item| item.text_content() == "committed before reply loss").count(), 1);
        let xai_acp_lib::AcpClientMessage::ExtNotification(args) = live.try_recv().unwrap() else { panic!("ambiguous receipt"); };
        assert_eq!(args.request.method.as_ref(), "x.ai/session/request_sources/unconfirmed");
        actor.chat_state_handle.push_user_message(ConversationItem::user("next actual turn"));
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "reconcile".into()).await.unwrap();
        let prepare = actor.prepare_request_for_sampling(built, None);
        let reconcile = async {
            let message @ PersistenceMsg::ReadPublishedRequestSources { .. } = rx.recv().await.unwrap() else { panic!("ambiguity requires authoritative journal reconciliation"); };
            persistence.tx.send(message).unwrap();
        };
        let (request, ()) = tokio::join!(prepare, reconcile);
        assert_eq!(request.items.iter().filter(|item| item.text_content() == "committed before reply loss").count(), 1);
        assert!(request.items.iter().any(|item| item.text_content() == "next actual turn"));
        assert!(rx.try_recv().is_err(), "no second publication append for the same occurrence");
        assert!(live.try_recv().is_err());
        let journal = std::fs::read_to_string(directory.path().join(crate::session::storage::UPDATES_FILE)).unwrap();
        assert_eq!(journal.lines().filter(|line| line.contains("committed before reply loss")).count(), 1);
    }).await;
}

#[tokio::test]
async fn definite_append_failure_then_restart_publishes_original_source_once() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    use crate::session::request_capture::SourcePublications;
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut live) = tokio::sync::mpsc::unbounded_channel();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut actor = create_test_actor(0, 256_000, 85, gateway, tx).await;
        let directory = tempfile::tempdir().unwrap();
        let calls = AtomicUsize::new(0);
        let storage = Arc::new(crate::session::storage::jsonl::JsonlStorageAdapter::with_update_append_probe(
            directory.path().to_owned(), move |_| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(std::io::Error::from(std::io::ErrorKind::StorageFull))
                } else { Ok(()) }
            },
        ));
        storage.init_session(&actor.session_info, acp::ModelId::new("test")).await.unwrap();
        let info = actor.session_info.clone();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage);
        actor.notifications.persistence_tx = persistence.handle.tx.clone();
        actor.push_system_reminder("survives definite failure and restart");
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "before-crash".into()).await.unwrap();
        let failed_request = actor.prepare_request_for_sampling(built, None).await;
        assert_eq!(failed_request.items.iter().filter(|item| item.text_content() == "survives definite failure and restart").count(), 1);
        let xai_acp_lib::AcpClientMessage::ExtNotification(failure) = live.try_recv().unwrap() else { panic!("definite failure diagnostic"); };
        let failure: crate::session::request_capture::SourcePublicationFailure = serde_json::from_str(failure.request.params.get()).unwrap();
        assert!(matches!(failure.progress, crate::session::request_capture::PublicationProgress::NotCommitted));
        let source = failure.sources[0];
        assert!(crate::session::storage::request_capture::published_sources(directory.path()).unwrap().is_empty());
        let (respond_to, response) = tokio::sync::oneshot::channel();
        persistence.handle.tx.send(PersistenceMsg::AppendUpdateDurablyAndAck {
            update: SessionUpdate::Acp(Box::new(acp::SessionNotification::new(
                info.id.clone(), acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    acp::ContentBlock::Text(acp::TextContent::new("primary persistence remains available")),
                )),
            ))), respond_to,
        }).unwrap();
        response.await.unwrap().unwrap();
        let checkpoint = actor.chat_state_handle.snapshot().await.unwrap();
        drop(actor);
        let (gateway, mut restarted_live) = tokio::sync::mpsc::unbounded_channel();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut restarted = create_test_actor(0, 256_000, 85, gateway, tx).await;
        restarted.session_info = info;
        restarted.notifications.persistence_tx = persistence.handle.tx.clone();
        *restarted.notifications.source_publications.lock() = SourcePublications::restored();
        restarted.chat_state_handle.restore_snapshot(checkpoint);
        restarted.chat_state_handle.push_user_message(ConversationItem::user("new human turn after restart"));
        let built = restarted.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "after-crash".into()).await.unwrap();
        let request = restarted.prepare_request_for_sampling(built, None).await;
        assert_eq!(request.items.iter().filter(|item| item.text_content() == "survives definite failure and restart").count(), 1);
        assert!(request.items.iter().any(|item| item.text_content() == "new human turn after restart"));
        let xai_acp_lib::AcpClientMessage::ExtNotification(applied) = restarted_live.try_recv().unwrap() else { panic!("original source must finally publish"); };
        let applied: XaiSessionNotification = serde_json::from_str(applied.request.params.get()).unwrap();
        let applied: crate::session::request_capture::SourcePublication = serde_json::from_value(applied.meta.unwrap()[crate::session::request_capture::SOURCE_PUBLICATION_META].clone()).unwrap();
        assert_eq!(applied.source, source);
        assert_eq!(crate::session::storage::request_capture::published_sources(directory.path()).unwrap(), vec![source.id().unwrap()]);
        assert!(restarted_live.try_recv().is_err());
        persistence.task.abort();
    }).await;
}

#[tokio::test]
async fn partial_publication_delivers_only_synced_prefix_then_retries_only_unapplied_suffix() {
    use crate::session::request_capture::{PublicationFailure, PublicationProgress, PublicationDurability, CaptureFailure};
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut live) = tokio::sync::mpsc::unbounded_channel();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway, tx).await;
        actor.push_system_reminder("prefix source");
        actor.push_system_reminder("suffix source");
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "partial".into()).await.unwrap();
        let expected = serde_json::to_value(&built.request.items).unwrap();
        let publish = actor.prepare_request_for_sampling(built, None);
        let commit_prefix = async {
            let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = rx.recv().await.unwrap() else { panic!("ordered batch"); };
            assert_eq!(updates.len(), 2);
            let ids: Vec<_> = updates.iter().map(|update| {
                let SessionUpdate::Xai(notification) = update else { panic!("source row"); };
                let publication: crate::session::request_capture::SourcePublication = serde_json::from_value(notification.meta.as_ref().unwrap()[crate::session::request_capture::SOURCE_PUBLICATION_META].clone()).unwrap();
                publication.source.id().unwrap()
            }).collect();
            respond_to.send(Err(PublicationFailure {
                cause: CaptureFailure::Storage,
                progress: PublicationProgress::JournalPrefix { committed: 1, durability: PublicationDurability::Synced },
            })).unwrap();
            ids
        };
        let (request, ids) = tokio::join!(publish, commit_prefix);
        assert_eq!(serde_json::to_value(&request.items).unwrap(), expected);
        let xai_acp_lib::AcpClientMessage::ExtNotification(prefix) = live.try_recv().unwrap() else { panic!("durable prefix"); };
        assert_eq!(prefix.request.method.as_ref(), "x.ai/session_notification");
        let prefix: XaiSessionNotification = serde_json::from_str(prefix.request.params.get()).unwrap();
        assert!(matches!(prefix.update, XaiSessionUpdate::InjectedContextRecord { text, .. } if text == "prefix source"));
        let xai_acp_lib::AcpClientMessage::ExtNotification(failure) = live.try_recv().unwrap() else { panic!("partial receipt"); };
        assert_eq!(failure.request.method.as_ref(), "x.ai/session/request_sources/unconfirmed");
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "suffix-retry".into()).await.unwrap();
        let prepare = actor.prepare_request_for_sampling(built, None);
        let retry = async {
            let PersistenceMsg::ReadPublishedRequestSources { respond_to } = rx.recv().await.unwrap() else { panic!("partial journal reconciliation"); };
            respond_to.send(Ok(vec![ids[0]])).unwrap();
            let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = rx.recv().await.unwrap() else { panic!("unapplied suffix"); };
            assert_eq!(updates.len(), 1);
            let SessionUpdate::Xai(notification) = &updates[0] else { panic!("suffix row"); };
            let publication: crate::session::request_capture::SourcePublication = serde_json::from_value(notification.meta.as_ref().unwrap()[crate::session::request_capture::SOURCE_PUBLICATION_META].clone()).unwrap();
            assert_eq!(publication.source.id(), Some(ids[1]));
            respond_to.send(Ok(())).unwrap();
        };
        let (request, ()) = tokio::join!(prepare, retry);
        assert_eq!(serde_json::to_value(&request.items).unwrap(), expected);
        let xai_acp_lib::AcpClientMessage::ExtNotification(suffix) = live.try_recv().unwrap() else { panic!("suffix publication"); };
        let suffix: XaiSessionNotification = serde_json::from_str(suffix.request.params.get()).unwrap();
        assert!(matches!(suffix.update, XaiSessionUpdate::InjectedContextRecord { text, .. } if text == "suffix source"));
        assert!(live.try_recv().is_err());
        assert!(rx.try_recv().is_err());
    }).await;
}

#[tokio::test]
async fn corrupt_publication_ledger_is_not_silently_defaulted_or_repeatedly_announced() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut live) = tokio::sync::mpsc::unbounded_channel();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway, tx).await;
        actor.push_system_reminder("corrupt ledger leaves model input intact");
        *actor.notifications.source_publications.lock() = crate::session::request_capture::SourcePublications::restored();
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "corrupt-ledger".into()).await.unwrap();
        let prepare = actor.prepare_request_for_sampling(built, None);
        let corrupt = async {
            let PersistenceMsg::ReadPublishedRequestSources { respond_to } = rx.recv().await.unwrap() else { panic!("lazy ledger restore"); };
            respond_to.send(Err(std::io::Error::from(std::io::ErrorKind::InvalidData))).unwrap();
        };
        let (request, ()) = tokio::join!(prepare, corrupt);
        let expected = serde_json::to_value(&request.items).unwrap();
        let xai_acp_lib::AcpClientMessage::ExtNotification(args) = live.try_recv().unwrap() else { panic!("typed corruption warning"); };
        let failure: crate::session::request_capture::SourcePublicationFailure = serde_json::from_str(args.request.params.get()).unwrap();
        assert!(matches!(failure.cause, crate::session::request_capture::CaptureFailure::InvalidEvidence));
        assert_eq!(failure.sources.len(), 1);
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "corruption-still-present".into()).await.unwrap();
        let request = actor.prepare_request_for_sampling(built, None).await;
        assert_eq!(serde_json::to_value(&request.items).unwrap(), expected);
        assert!(rx.try_recv().is_err(), "corruption is not retried as transient I/O or converted to an empty ledger");
        assert!(live.try_recv().is_err(), "unchanged corrupt evidence emits one warning");
    }).await;
}

#[tokio::test]
async fn changed_restore_cause_reannounces_while_identical_cause_stays_suppressed() {
    tokio::task::LocalSet::new().run_until(async {
        use crate::session::request_capture::CaptureFailure;
        let (gateway, mut live) = tokio::sync::mpsc::unbounded_channel();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway, tx).await;
        actor.push_system_reminder("restore cause visibility");
        *actor.notifications.source_publications.lock() = crate::session::request_capture::SourcePublications::restored();
        let restore = |cause| async {
            let PersistenceMsg::ReadPublishedRequestSources { respond_to } = rx.recv().await.unwrap() else { panic!("transient restore retry"); };
            respond_to.send(Err(std::io::Error::from(cause))).unwrap();
        };
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "first-failure".into()).await.unwrap();
        let (request, ()) = tokio::join!(actor.prepare_request_for_sampling(built, None), restore(std::io::ErrorKind::StorageFull));
        assert!(request.items.iter().any(|item| item.text_content() == "restore cause visibility"));
        let xai_acp_lib::AcpClientMessage::ExtNotification(first) = live.try_recv().unwrap() else { panic!("first restore failure warning"); };
        let first: crate::session::request_capture::SourcePublicationFailure = serde_json::from_str(first.request.params.get()).unwrap();
        assert!(matches!(first.cause, CaptureFailure::Io { kind: crate::session::extensions::context_policy::ContextIoKind::StorageFull, .. }));
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "identical-failure".into()).await.unwrap();
        let (_, ()) = tokio::join!(actor.prepare_request_for_sampling(built, None), restore(std::io::ErrorKind::StorageFull));
        assert!(live.try_recv().is_err(), "identical cause stays suppressed");
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "changed-failure".into()).await.unwrap();
        let (request, ()) = tokio::join!(actor.prepare_request_for_sampling(built, None), restore(std::io::ErrorKind::PermissionDenied));
        assert!(request.items.iter().any(|item| item.text_content() == "restore cause visibility"));
        let xai_acp_lib::AcpClientMessage::ExtNotification(changed) = live.try_recv().unwrap() else { panic!("changed cause is reannounced"); };
        let changed: crate::session::request_capture::SourcePublicationFailure = serde_json::from_str(changed.request.params.get()).unwrap();
        assert!(matches!(changed.cause, CaptureFailure::Io { kind: crate::session::extensions::context_policy::ContextIoKind::PermissionDenied, .. }));
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "recovered".into()).await.unwrap();
        let prepare = actor.prepare_request_for_sampling(built, None);
        let recover = async {
            let PersistenceMsg::ReadPublishedRequestSources { respond_to } = rx.recv().await.unwrap() else { panic!("final restore retry"); };
            respond_to.send(Ok(vec![])).unwrap();
            let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = rx.recv().await.unwrap() else { panic!("recovered publication"); };
            assert_eq!(updates.len(), 1);
            respond_to.send(Ok(())).unwrap();
        };
        let (request, ()) = tokio::join!(prepare, recover);
        assert_eq!(request.items.iter().filter(|item| item.text_content() == "restore cause visibility").count(), 1);
        let xai_acp_lib::AcpClientMessage::ExtNotification(applied) = live.try_recv().unwrap() else { panic!("publication after recovery"); };
        assert_eq!(applied.request.method.as_ref(), "x.ai/session_notification");
        assert!(live.try_recv().is_err());
        assert!(rx.try_recv().is_err());
    }).await;
}

#[tokio::test]
async fn closed_publication_mailbox_is_not_submission_and_reopened_resource_retries_without_reconciliation() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut live) = tokio::sync::mpsc::unbounded_channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut actor = create_test_actor(0, 256_000, 85, gateway, tx).await;
        drop(rx);
        actor.push_system_reminder("not submitted to closed mailbox");
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "closed".into()).await.unwrap();
        actor.prepare_request_for_sampling(built, None).await;
        let xai_acp_lib::AcpClientMessage::ExtNotification(args) = live.try_recv().unwrap() else { panic!("non-submission diagnostic"); };
        let failure: crate::session::request_capture::SourcePublicationFailure = serde_json::from_str(args.request.params.get()).unwrap();
        assert!(matches!(failure.cause, crate::session::request_capture::CaptureFailure::MailboxClosed));
        assert!(matches!(failure.progress, crate::session::request_capture::PublicationProgress::NotSubmitted));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        actor.notifications.persistence_tx = tx;
        let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), "resource-reopened".into()).await.unwrap();
        let prepare = actor.prepare_request_for_sampling(built, None);
        let retry = async {
            let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = rx.recv().await.unwrap() else { panic!("failed send did not apply; direct retry is safe"); };
            assert_eq!(updates.len(), 1);
            respond_to.send(Ok(())).unwrap();
        };
        let (request, ()) = tokio::join!(prepare, retry);
        assert_eq!(request.items.iter().filter(|item| item.text_content() == "not submitted to closed mailbox").count(), 1);
        let xai_acp_lib::AcpClientMessage::ExtNotification(args) = live.try_recv().unwrap() else { panic!("retried publication"); };
        assert_eq!(args.request.method.as_ref(), "x.ai/session_notification");
        assert!(rx.try_recv().is_err());
        assert!(live.try_recv().is_err());
    }).await;
}
