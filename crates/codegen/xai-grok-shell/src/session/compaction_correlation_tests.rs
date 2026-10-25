use super::*;
use crate::extensions::notification::{SessionNotification, SessionUpdate};
use crate::session::acp_types::CompactInvocationId;
use crate::session::acp_session::support;

#[derive(Clone, Copy)]
enum AckDisposition {
    Persist,
    Disconnect,
}

#[derive(Clone, Copy)]
enum Teardown {
    Shutdown,
    ChannelClosed,
}

#[derive(Clone, Copy)]
enum ShutdownPhase {
    Queued,
    Reserved,
    Generating,
}

#[derive(Clone, Copy)]
enum Outcome {
    Success,
    ReservedCancellation,
    GeneratingCancellation,
    Failure,
}

#[tokio::test(flavor = "current_thread")]
async fn original_manual_worker_publishes_exact_invocation_metadata() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            for outcome in [Outcome::Success, Outcome::ReservedCancellation,
                Outcome::GeneratingCancellation, Outcome::Failure] {
                let (gateway, mut gateway_events) = mpsc::unbounded_channel();
                let (notices, mut received) = mpsc::unbounded_channel();
                tokio::task::spawn_local(async move {
                    while let Some(message) = gateway_events.recv().await {
                        match message {
                            xai_acp_lib::AcpClientMessage::SessionNotification(args) => {
                                let _ = args.response_tx.send(Ok(()));
                            }
                            xai_acp_lib::AcpClientMessage::ExtNotification(args) => {
                                if args.request.method.as_ref() == "x.ai/session_notification" {
                                    let notification: SessionNotification = serde_json::from_str(args.request.params.get()).unwrap();
                                    let _ = notices.send(notification);
                                }
                            }
                            _ => {}
                        }
                    }
                });
                let (mut actor, events, _writes) = support::create_observed_completion_test_actor(
                    50_000, 200_000, 85, gateway,
                ).await;
                let sampling_events = wire_sampler(&mut actor).await;
                let actor = Arc::new(actor);
                actor.chat_state_handle.replace_conversation(vec![
                    ConversationItem::system("sys"),
                    ConversationItem::user("original task"),
                    ConversationItem::assistant("Original completed work. ".repeat(40)),
                    ConversationItem::user("final query"),
                ]);
                let server = route_to_mock(&actor).await;
                let reservation = match outcome {
                    Outcome::ReservedCancellation => {
                        let (reached, reserved) = oneshot::channel();
                        let (release, released) = oneshot::channel();
                        actor.compaction.cancel.manual_reservation_pause.replace(Some((reached, released)));
                        Some((reserved, release))
                    }
                    Outcome::GeneratingCancellation => { server.set_auxiliary_hold(); None }
                    Outcome::Failure => {
                        server.set_conversations(vec![xai_grok_test_support::Conversation::nth(1)
                            .refuse(xai_grok_test_support::StatusFailure::new(400).with_body("invalid request"))]);
                        None
                    }
                    Outcome::Success => None,
                };
                let (sender, _task) = start_fixture_actor(actor.clone(), events, sampling_events);
                let id = CompactInvocationId(uuid::Uuid::new_v4());
                let cancellation = tokio_util::sync::CancellationToken::new();
                let (respond_to, response) = oneshot::channel();
                sender.send(SessionCommand::CompactSession {
                    invocation_id: Some(id), cancellation: cancellation.clone(), respond_to,
                }).unwrap();
                if let Some((reserved, release)) = reservation {
                    reserved.await.unwrap();
                    assert_eq!(actor.compaction.cancel.pending_manual.borrow().front().unwrap().invocation_id, Some(id));
                    cancellation.cancel();
                    let _ = release.send(());
                }
                if matches!(outcome, Outcome::GeneratingCancellation) {
                    server.wait_until_parked_replies(1).await;
                    cancellation.cancel();
                }
                let result = response.await.unwrap();
                match outcome {
                    Outcome::Success => assert!(result.is_ok(), "real persistence must ACK the original worker: {result:?}"),
                    Outcome::ReservedCancellation | Outcome::GeneratingCancellation => {
                        assert_eq!(crate::session::helpers::session_compact::compact_error_kind(&result.unwrap_err()),
                            Some(crate::session::helpers::session_compact::CompactErrorKind::Cancelled));
                    }
                    Outcome::Failure => assert!(result.is_err()),
                }
                loop {
                    let notification = received.recv().await.unwrap();
                    if matches!(notification.update, SessionUpdate::AutoCompactCompleted { .. }
                        | SessionUpdate::AutoCompactCancelled { .. } | SessionUpdate::AutoCompactFailed { .. }) {
                        assert_eq!(notification.meta.as_ref().unwrap()["compactInvocationId"], serde_json::json!(id));
                        assert!(notification.meta.as_ref().unwrap().get("eventId").is_some());
                        match outcome {
                            Outcome::Success => assert!(matches!(notification.update, SessionUpdate::AutoCompactCompleted { .. })),
                            Outcome::ReservedCancellation | Outcome::GeneratingCancellation => {
                                assert!(matches!(notification.update, SessionUpdate::AutoCompactCancelled { .. }));
                            }
                            Outcome::Failure => assert!(matches!(notification.update, SessionUpdate::AutoCompactFailed { .. })),
                        }
                        break;
                    }
                }
                server.release_parked_replies();
            }
        }).await.expect("original correlated worker must settle through real persistence");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn correlated_original_responder_waits_for_own_ack_and_reports_transport_loss() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            for disposition in [AckDisposition::Persist, AckDisposition::Disconnect] {
                let (gateway, messages) = mpsc::unbounded_channel();
                support::drain_gateway(messages);
                let (mut actor, events, _writes) = support::create_observed_completion_test_actor(
                    50_000, 200_000, 85, gateway,
                ).await;
                let sampling_events = wire_sampler(&mut actor).await;
                let persistence = actor.notifications.persistence_tx.clone();
                let (relay, mut messages) = mpsc::unbounded_channel();
                let (held, mut acknowledgements) = mpsc::unbounded_channel();
                let (notices, mut notifications) = mpsc::unbounded_channel();
                let disk = persistence.clone();
                let relay_task = tokio::task::spawn_local(async move {
                    while let Some(message) = messages.recv().await {
                        if let PersistenceMsg::Update(crate::session::storage::SessionUpdate::Xai(notification)) = &message {
                            notices.send(notification.clone()).unwrap();
                        }
                        match message {
                            message @ PersistenceMsg::FlushAndAck { .. } => held.send(message).unwrap(),
                            message => disk.send(message).unwrap(),
                        }
                    }
                });
                actor.notifications.persistence_tx = relay;
                let actor = Arc::new(actor);
                actor.chat_state_handle.replace_conversation(vec![
                    ConversationItem::system("sys"), ConversationItem::user("original task"),
                    ConversationItem::assistant("Original completed work. ".repeat(40)),
                    ConversationItem::user("final query"),
                ]);
                let server = route_to_mock(&actor).await;
                let (sender, _task) = start_fixture_actor(actor.clone(), events, sampling_events);
                let id = CompactInvocationId(uuid::Uuid::new_v4());
                let (respond_to, mut response) = oneshot::channel();
                sender.send(SessionCommand::CompactSession {
                    invocation_id: Some(id), cancellation: tokio_util::sync::CancellationToken::new(), respond_to,
                }).unwrap();
                let own_ack = acknowledgements.recv().await.unwrap();
                assert!(actor.compaction.cancel.is_committing());
                assert!(actor.state.lock().await.finalization_gate.is_active());
                assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                loop {
                    let notification = notifications.recv().await.unwrap();
                    if matches!(notification.update, SessionUpdate::AutoCompactCompleted { .. }) {
                        assert_eq!(notification.meta.as_ref().unwrap()["compactInvocationId"], serde_json::json!(id));
                        break;
                    }
                }
                sender.send(SessionCommand::Shutdown(crate::session::ShutdownKind::Graceful)).unwrap();
                actor.compaction.cancel.manual_teardown_started.notified().await;
                assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                assert!(!_task.is_finished(), "shutdown cannot release the original responder before its own ACK");
                match disposition {
                    AckDisposition::Persist => persistence.send(own_ack).unwrap(),
                    AckDisposition::Disconnect => {
                        let PersistenceMsg::FlushAndAck { respond_to } = own_ack else { unreachable!() };
                        let (disk_reply, disk_ack) = oneshot::channel();
                        persistence.send(PersistenceMsg::FlushAndAck { respond_to: disk_reply }).unwrap();
                        disk_ack.await.unwrap().expect("the real persistence owner must flush before its receipt is lost");
                        assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                        drop(respond_to);
                    }
                }
                let result = response.await.unwrap();
                match disposition {
                    AckDisposition::Persist => assert!(result.is_ok(), "real persistence must answer the original ACK: {result:?}"),
                    AckDisposition::Disconnect => {
                        assert_eq!(crate::sampling::error::acp_error_message(&result.unwrap_err()),
                            "compaction persistence did not respond");
                        loop {
                            let notification = notifications.recv().await.unwrap();
                            if matches!(notification.update, SessionUpdate::AutoCompactFailed { .. }) {
                                assert_eq!(notification.meta.as_ref().unwrap()["compactInvocationId"], serde_json::json!(id));
                                break;
                            }
                        }
                    }
                }
                let mut task = _task;
                loop {
                    tokio::select! {
                        result = &mut task => { result.unwrap(); break; }
                        message = acknowledgements.recv() => persistence.send(message.unwrap()).unwrap(),
                    }
                }
                assert!(!actor.compaction.cancel.is_active());
                assert!(!actor.state.lock().await.finalization_gate.is_active());
                assert_eq!(server.requests().iter().filter(|request| request.conversation.is_some()).count(), 0);
                relay_task.abort();
            }
        }).await.expect("correlated own-ACK settlement must remain owned through shutdown");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn correlated_shutdown_cancels_original_queued_reserved_and_generating_requests() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            for (phase, teardown) in [ShutdownPhase::Queued, ShutdownPhase::Reserved, ShutdownPhase::Generating]
                .into_iter().flat_map(|phase| [Teardown::Shutdown, Teardown::ChannelClosed].map(|end| (phase, end))) {
                let (gateway, messages) = mpsc::unbounded_channel();
                support::drain_gateway(messages);
                let (mut actor, events, _writes) = support::create_observed_completion_test_actor(
                    50_000, 200_000, 85, gateway,
                ).await;
                let sampling_events = wire_sampler(&mut actor).await;
                let persistence = actor.notifications.persistence_tx.clone();
                let (relay, mut messages) = mpsc::unbounded_channel();
                let (notices, mut notifications) = mpsc::unbounded_channel();
                let relay_task = tokio::task::spawn_local(async move {
                    while let Some(message) = messages.recv().await {
                        if let PersistenceMsg::Update(crate::session::storage::SessionUpdate::Xai(notification)) = &message {
                            notices.send(notification.clone()).unwrap();
                        }
                        persistence.send(message).unwrap();
                    }
                });
                actor.notifications.persistence_tx = relay;
                let actor = Arc::new(actor);
                actor.chat_state_handle.replace_conversation(vec![
                    ConversationItem::system("sys"), ConversationItem::user("original task"),
                    ConversationItem::assistant("Original completed work. ".repeat(40)),
                    ConversationItem::user("final query"),
                ]);
                let server = route_to_mock(&actor).await;
                let reservation = match phase {
                    ShutdownPhase::Queued => {
                        actor.state.lock().await.running_task = Some(running_task_stub("foreground"));
                        None
                    }
                    ShutdownPhase::Reserved => {
                        let (reached, reserved) = oneshot::channel();
                        let (release, released) = oneshot::channel();
                        actor.compaction.cancel.manual_reservation_pause.replace(Some((reached, released)));
                        Some((reserved, release))
                    }
                    ShutdownPhase::Generating => { server.set_auxiliary_hold(); None }
                };
                let (mut sender, task) = start_fixture_actor(actor.clone(), events, sampling_events);
                let id = CompactInvocationId(uuid::Uuid::new_v4());
                let (respond_to, mut response) = oneshot::channel();
                sender.send(SessionCommand::CompactSession {
                    invocation_id: Some(id), cancellation: tokio_util::sync::CancellationToken::new(), respond_to,
                }).unwrap();
                let release = if let Some((reserved, release)) = reservation {
                    reserved.await.unwrap();
                    assert_eq!(actor.compaction.cancel.pending_manual.borrow().front().unwrap().invocation_id, Some(id));
                    Some(release)
                } else {
                    match phase {
                        ShutdownPhase::Queued => loop {
                            let notification = notifications.recv().await.unwrap();
                            if matches!(notification.update, SessionUpdate::AutoCompactStarted { phase: crate::extensions::notification::CompactPhase::Queued, .. }) {
                                assert_eq!(notification.meta.as_ref().unwrap()["compactInvocationId"], serde_json::json!(id));
                                break;
                            }
                        },
                        ShutdownPhase::Generating => server.wait_until_parked_replies(1).await,
                        ShutdownPhase::Reserved => unreachable!(),
                    }
                    None
                };
                assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                match teardown {
                    Teardown::Shutdown => sender.send(SessionCommand::Shutdown(crate::session::ShutdownKind::Graceful)).unwrap(),
                    Teardown::ChannelClosed => {
                        let (replacement, _receiver) = mpsc::unbounded_channel();
                        drop(std::mem::replace(&mut sender, replacement));
                    }
                }
                actor.compaction.cancel.manual_teardown_started.notified().await;
                let error = response.await.unwrap().unwrap_err();
                assert_eq!(crate::session::helpers::session_compact::compact_error_kind(&error),
                    Some(crate::session::helpers::session_compact::CompactErrorKind::Cancelled));
                loop {
                    let notification = notifications.recv().await.unwrap();
                    if matches!(notification.update, SessionUpdate::AutoCompactCancelled { .. }) {
                        assert_eq!(notification.meta.as_ref().unwrap()["compactInvocationId"], serde_json::json!(id));
                        break;
                    }
                }
                if let Some(release) = release {
                    assert!(!task.is_finished(), "reserved worker still owns teardown until released");
                    release.send(()).unwrap();
                }
                task.await.unwrap();
                assert!(actor.compaction.cancel.manual_is_sealed());
                assert!(!actor.has_pending_compact());
                assert!(!actor.compaction.cancel.is_active());
                assert!(!actor.state.lock().await.finalization_gate.is_active());
                assert_eq!(server.requests().iter().filter(|request| request.conversation.is_some()).count(), 0);
                server.release_parked_replies();
                relay_task.abort();
            }
        }).await.expect("tagged requests must retain their original cancellation through teardown");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn queued_manual_cancellation_persists_the_original_invocation_id() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut fixture = fixture().await;
            fixture.actor.state.lock().await.running_task = Some(running_task_stub("foreground"));
            let id = CompactInvocationId(uuid::Uuid::new_v4());
            let cancellation = tokio_util::sync::CancellationToken::new();
            let (respond_to, response) = oneshot::channel();
            fixture.sender.send(SessionCommand::CompactSession {
                invocation_id: Some(id), cancellation: cancellation.clone(), respond_to,
            }).unwrap();
            command_boundary(&fixture).await;
            assert_eq!(fixture.actor.compaction.cancel.pending_manual.borrow().front().unwrap().invocation_id, Some(id));
            cancellation.cancel();
            assert!(response.await.unwrap().is_err());
            let mut phases = Vec::new();
            while phases.len() < 2 {
                if let PersistenceMsg::Update(crate::session::storage::SessionUpdate::Xai(notification)) = fixture.persistence.recv().await.unwrap() {
                    match notification.update {
                        SessionUpdate::AutoCompactStarted { phase, .. } => {
                            assert_eq!(phase, crate::extensions::notification::CompactPhase::Queued);
                            phases.push("queued");
                        }
                        SessionUpdate::AutoCompactCancelled { .. } => phases.push("cancelled"),
                        _ => continue,
                    }
                    assert_eq!(notification.meta.as_ref().unwrap()["compactInvocationId"], serde_json::json!(id));
                    let envelope = crate::session::storage::SessionUpdateEnvelope::from_update(
                        &crate::session::storage::SessionUpdate::Xai(notification),
                    ).unwrap();
                    let line = serde_json::to_string(&envelope).unwrap();
                    let crate::session::storage::SessionUpdate::Xai(decoded) =
                        crate::session::storage::SessionUpdateEnvelope::from_str(&line).unwrap() else {
                            panic!("manual metadata must survive the persisted envelope");
                        };
                    assert_eq!(decoded.meta.as_ref().unwrap()["compactInvocationId"], serde_json::json!(id));
                }
            }
            assert_eq!(phases, vec!["queued", "cancelled"]);
        }).await.expect("queued cancellation must persist both correlated lifecycle updates");
    }).await;
}
