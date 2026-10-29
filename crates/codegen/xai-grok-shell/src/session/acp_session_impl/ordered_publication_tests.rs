use super::*;
use crate::session::acp_session::replay_buffer_send_update_tests::make_replay_send_update_fixture;
use crate::session::current_actions::{CurrentActionsSnapshot, SnapshotBinding};

#[tokio::test(flavor = "current_thread")]
async fn durable_producer_enrollment_survives_allocation_persistence_and_held_receiver() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut fixture = make_replay_send_update_fixture().await;
            let (tx, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            let gateway = xai_acp_lib::AcpAgentGatewaySender::new(tx)
                .with_local_receiver_settlement()
                .unwrap();
            fixture.actor.notifications.gateway = gateway.clone();
            fixture.actor.current_actions.attach_emitter(
                gateway.clone(),
                fixture.actor.notifications.gateway_enabled.clone(),
                fixture.actor.session_info.id.clone(),
            );
            let binding = SnapshotBinding {
                session_id: fixture.actor.session_info.id.0.to_string(),
                binding_epoch: 2,
                load_generation: 8,
            };
            let reader = fixture.actor.current_actions_reader();
            reader.bind_publication(binding.clone()).unwrap();
            let publish = fixture.actor.send_xai_notification_with_extra_meta(
                XaiSessionUpdate::HookAnnotation {
                    message: "actual delayed producer".into(),
                    kind: Default::default(),
                },
                None,
                crate::session::storage::jsonl::AppendDurability::Durable,
            );
            tokio::pin!(publish);
            assert!(futures::poll!(&mut publish).is_pending());
            let PersistenceMsg::AppendUpdateDurablyAndAck { update, respond_to } =
                fixture.persistence_rx.recv().await.unwrap()
            else {
                panic!("real durable producer must be waiting for persistence");
            };
            let crate::session::storage::SessionUpdate::Xai(saved) = update else {
                panic!("real xAI record");
            };
            let event_id = saved.meta.as_ref().unwrap().get("eventId").unwrap().clone();
            let snapshot = reader.snapshot(binding.clone());
            let CurrentActionsSnapshot::Current(before) = &snapshot else {
                panic!("actual actor reader");
            };
            let producers = gateway.producer_prefix();
            let fence = async {
                producers.settle().await?;
                gateway.delivery_prefix().settle().await
            };
            tokio::pin!(fence);
            assert!(
                futures::poll!(&mut fence).is_pending(),
                "allocated and persisted-enqueued is not published"
            );
            assert!(receiver.try_recv().is_err());
            respond_to.send(Ok(())).unwrap();
            publish.await.unwrap();
            let xai_acp_lib::AcpClientMessage::ExtNotification(record) =
                receiver.recv().await.unwrap()
            else {
                panic!("actual delayed publication");
            };
            let delivered: serde_json::Value =
                serde_json::from_str(record.request.params.get()).unwrap();
            assert_eq!(delivered, serde_json::to_value(&*saved).unwrap());
            assert_eq!(delivered["_meta"]["eventId"], event_id);
            assert!(
                futures::poll!(&mut fence).is_pending(),
                "gateway enqueue is not receiver processing"
            );
            record.response_tx.send(Ok(())).unwrap();
            fence.await.unwrap();
            let CurrentActionsSnapshot::Current(after) = reader.snapshot(binding) else {
                panic!("same source");
            };
            assert_eq!(after.version, before.version);
            assert_eq!(after.turn, before.turn);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn stale_queued_record_needs_actual_receiver_discard_completion() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut fixture = make_replay_send_update_fixture().await;
            let (tx, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            let gateway = xai_acp_lib::AcpAgentGatewaySender::new(tx)
                .with_local_receiver_settlement()
                .unwrap();
            fixture.actor.notifications.gateway = gateway.clone();
            fixture.actor.current_actions.attach_emitter(
                gateway.clone(),
                fixture.actor.notifications.gateway_enabled.clone(),
                fixture.actor.session_info.id.clone(),
            );
            *fixture.actor.current_prompt_id.lock().unwrap() = Some("first-prompt".into());
            fixture.actor.begin_sampling_turn();
            let request = xai_grok_sampler::RequestId::random();
            fixture.actor.register_sampling_request(&request);
            fixture
                .actor
                .send_sampling_update(
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                        acp::ContentBlock::from("original queued content"),
                    )),
                    None,
                    &request,
                )
                .await;
            let SessionEvent::Notification(original) = fixture.event_rx.recv().await.unwrap()
            else {
                panic!("actual native record must be queued");
            };
            let saved = serde_json::to_value(&original).unwrap();
            *fixture.actor.current_prompt_id.lock().unwrap() = Some("later-prompt".into());
            fixture.actor.begin_sampling_turn();
            fixture.actor.emit_buffered(original).await;
            let xai_acp_lib::AcpClientMessage::ExtNotification(discard) =
                receiver.recv().await.unwrap()
            else {
                panic!("stale record must reach the actual recipient as an explicit discard");
            };
            assert_eq!(
                discard.request.method.as_ref(),
                crate::extensions::current_actions::DISCARD_METHOD
            );
            let body: crate::extensions::current_actions::DiscardedRecord =
                serde_json::from_str(discard.request.params.get()).unwrap();
            assert_eq!(serde_json::to_value(body.notification).unwrap(), saved);
            let prefix = gateway.delivery_prefix().settle();
            tokio::pin!(prefix);
            assert!(futures::poll!(&mut prefix).is_pending());
            discard.response_tx.send(Ok(())).unwrap();
            prefix.await.unwrap();
            assert_eq!(
                fixture.actor.current_prompt_id.lock().unwrap().as_deref(),
                Some("later-prompt")
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_native_tool_terminal_after_source_change_keeps_publication_debt() {
    use crate::extensions::tool_timing::{ToolCallOwnership, ToolCallSource};
    use crate::session::current_actions::{FrozenOwner, ToolRegistration, TurnRegistration};
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut fixture = make_replay_send_update_fixture().await;
            let (tx, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            let gateway = xai_acp_lib::AcpAgentGatewaySender::new(tx)
                .with_local_receiver_settlement()
                .unwrap();
            fixture.actor.notifications.gateway = gateway.clone();
            fixture.actor.current_actions.attach_emitter(
                gateway.clone(),
                fixture.actor.notifications.gateway_enabled.clone(),
                fixture.actor.session_info.id.clone(),
            );
            let actor = &fixture.actor;
            let now = xai_monotonic_clock::MonotonicPoint::at(tokio::time::Instant::now());
            actor.current_actions.start_turn(
                FrozenOwner::Root {
                    prompt_id: "original-prompt".into(),
                    task_id: "original-task".into(),
                },
                now,
            );
            let turn = TurnRegistration::new(actor.current_actions.clone(), "original-task".into());
            let ownership = ToolCallOwnership::Registered {
                invocation_id: "original-invocation".into(),
                provider_call_id: "original-provider".into(),
                source: ToolCallSource::Unknown,
            };
            let registration = turn
                .scope(async {
                    ToolRegistration::new(
                        actor.current_actions.clone(),
                        ownership,
                        xai_monotonic_clock::MonotonicPoint::at(tokio::time::Instant::now()),
                    )
                })
                .await;
            let binding = SnapshotBinding {
                session_id: actor.session_info.id.0.to_string(),
                binding_epoch: 6,
                load_generation: 1,
            };
            let reader = actor.current_actions_reader();
            reader.bind_publication(binding.clone()).unwrap();
            let CurrentActionsSnapshot::Current(before) = reader.snapshot(binding.clone()) else {
                panic!("actual actor source");
            };
            assert_eq!(before.tools.len(), 1);
            while let Ok(message) = receiver.try_recv() {
                let xai_acp_lib::AcpClientMessage::ExtNotification(record) = message else {
                    panic!("source transitions only");
                };
                record.response_tx.send(Ok(())).unwrap();
            }
            gateway.delivery_prefix().settle().await.unwrap();
            let state = actor.state.lock().await;
            let mut publish = Box::pin(registration.scope(actor.send_update(
                acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                    "original-provider",
                    acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
                )),
                None,
            )));
            assert!(
                futures::poll!(&mut publish).is_pending(),
                "actual rewind lock holds producer before queue admission"
            );
            let CurrentActionsSnapshot::Current(after) = reader.snapshot(binding) else {
                panic!("same source");
            };
            assert!(
                after.tools.is_empty(),
                "real terminal publication already retired the registered tool"
            );
            assert_eq!(after.turn, before.turn);
            assert_ne!(after.version, before.version);
            assert!(
                fixture.event_rx.try_recv().is_err(),
                "terminal has not reached SessionEvent"
            );
            let prefix = gateway.producer_prefix().settle();
            tokio::pin!(prefix);
            assert!(futures::poll!(&mut prefix).is_pending());
            drop(publish);
            drop(state);
            assert_eq!(
                prefix.await,
                Err(xai_acp_lib::PublicationFailure::ProducerLost)
            );
            assert_eq!(
                gateway.producer_prefix().settle().await,
                Err(xai_acp_lib::PublicationFailure::ProducerLost)
            );
            assert!(
                fixture.event_rx.try_recv().is_err(),
                "aborted producer cannot create a phantom native record"
            );
            while let Ok(message) = receiver.try_recv() {
                let xai_acp_lib::AcpClientMessage::ExtNotification(record) = message else {
                    panic!("no native terminal may escape");
                };
                assert_eq!(
                    record.request.method.as_ref(),
                    crate::session::current_actions::TRANSITION_METHOD
                );
                record.response_tx.send(Ok(())).unwrap();
            }
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_durable_producer_cannot_become_a_successful_prefix() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut fixture = make_replay_send_update_fixture().await;
            let (tx, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            let gateway = xai_acp_lib::AcpAgentGatewaySender::new(tx)
                .with_local_receiver_settlement()
                .unwrap();
            fixture.actor.notifications.gateway = gateway.clone();
            let mut publish = Box::pin(fixture.actor.send_xai_notification_with_extra_meta(
                XaiSessionUpdate::HookAnnotation {
                    message: "cancelled after allocation".into(),
                    kind: Default::default(),
                },
                None,
                crate::session::storage::jsonl::AppendDurability::Durable,
            ));
            assert!(futures::poll!(&mut publish).is_pending());
            let PersistenceMsg::AppendUpdateDurablyAndAck { respond_to, .. } =
                fixture.persistence_rx.recv().await.unwrap()
            else {
                panic!("actual persistence wait");
            };
            let prefix = gateway.producer_prefix();
            drop(publish);
            assert_eq!(
                prefix.settle().await,
                Err(xai_acp_lib::PublicationFailure::ProducerLost)
            );
            assert!(respond_to.send(Ok(())).is_err());
            assert!(receiver.try_recv().is_err());
        })
        .await;
}
