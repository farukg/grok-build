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
        let mut published = Vec::new();
        for (iteration, addition) in [Some("first injected body"), Some("second injected body"), None].into_iter().enumerate() {
            if let Some(body) = addition {
                actor.chat_state_handle.push_user_message(ConversationItem::system_reminder(body));
            }
            let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), format!("r-{iteration}")).await.unwrap();
            let actual_items = built.request.items.clone();
            let prepare = actor.prepare_request_for_sampling(built, None, &mut published);
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
                let request = request.unwrap();
                assert_eq!(serde_json::to_value(&request.items).unwrap(), serde_json::to_value(&actual_items).unwrap());
                assert!(request.items.iter().any(|item| item.text_content() == expected_body));
                assert!(gateway_rx.try_recv().is_ok());
                assert!(gateway_rx.try_recv().is_err());
            } else {
                let request = prepare.await.unwrap();
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
            let mut published = Vec::new();
            let mut event_ids = std::collections::HashSet::new();
            for iteration in 0..3 {
                if iteration < 2 {
                    actor.chat_state_handle.push_user_message(ConversationItem::system_reminder(body));
                }
                let built = actor.chat_state_handle.build_request(vec![], None, false, None, "c".into(), format!("equal-{iteration}")).await.unwrap();
                let expected_count = (iteration + 1).min(2);
                assert_eq!(built.items.iter().filter(|item| item.text_content() == body).count(), expected_count);
                assert_eq!(built.injected_records.iter().filter(|record| record.text() == body).count(), expected_count);
                let prepare = actor.prepare_request_for_sampling(built, None, &mut published);
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
                    request.unwrap()
                } else {
                    prepare.await.unwrap()
                };
                assert_eq!(request.items.iter().filter(|item| item.text_content() == body).count(), expected_count);
                assert_eq!(published.iter().filter(|record| record.text() == body).count(), expected_count);
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
            let mut published = Vec::new();
            let publish = actor.prepare_request_for_sampling(built, None, &mut published);
            let reject = async {
                let PersistenceMsg::AppendUpdatesAndAck { updates, respond_to } = persistence_rx.recv().await.unwrap() else {
                    panic!("expected acknowledged request batch");
                };
                assert_eq!(updates.len(), 1);
                assert!(gateway_rx.try_recv().is_err());
                if lose_ack { drop(respond_to); } else {
                    respond_to.send(Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "full"))).unwrap();
                }
            };
            let (result, ()) = tokio::join!(publish, reject);
            assert!(result.is_err(), "no owned sampler request may escape a failed batch");
            assert!(published.is_empty());
            assert!(gateway_rx.try_recv().is_err());
        }
    }).await;
}
