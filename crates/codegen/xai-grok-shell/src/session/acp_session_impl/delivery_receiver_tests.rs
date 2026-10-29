use super::*;
use crate::session::acp_session::support::{
    create_test_actor_with_chat_persistence, running_task_stub,
};
use crate::session::delivery_receiver::DeliveryAction;
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter};

fn notifications(
    receiver: &mut mpsc::UnboundedReceiver<xai_acp_lib::AcpClientMessage>,
) -> Vec<acp::ExtNotification> {
    std::iter::from_fn(|| receiver.try_recv().ok())
        .filter_map(|message| match message {
            xai_acp_lib::AcpClientMessage::ExtNotification(args) => Some(args.request),
            _ => None,
        })
        .collect()
}

fn queue_snapshots(
    notifications: &[acp::ExtNotification],
) -> Vec<crate::session::prompt_queue::QueueChanged> {
    notifications
        .iter()
        .filter(|notification| notification.method.as_ref() == "x.ai/queue/changed")
        .map(|notification| serde_json::from_str(notification.params.get()).unwrap())
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn safe_point_publishes_delivery_once_and_reconciles_queue_without_losing_user_input() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let directory = tempfile::tempdir().unwrap();
            let info = crate::session::info::Info {
                id: acp::SessionId::new("test-actor"),
                cwd: "/tmp".into(),
            };
            let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(
                directory.path().to_owned(),
            ));
            storage
                .init_session(&info, acp::ModelId::new("test"))
                .await
                .unwrap();
            let persistence = crate::session::persistence::durable_update_tests::test_actor(
                info.clone(),
                storage,
            );
            let mut persistence_task = tokio_util::task::AbortOnDropHandle::new(persistence.task);
            let (gateway, mut events) = mpsc::unbounded_channel();
            let (actor, _) = create_test_actor_with_chat_persistence(
                0,
                256_000,
                85,
                gateway,
                persistence.handle.tx.clone(),
                Box::new(
                    crate::session::chat_persistence::ChannelChatPersistence::new(
                        persistence.handle.tx.clone(),
                    ),
                ),
            )
            .await;
            actor.state.lock().await.running_task = Some(running_task_stub("running"));
            assert!(!actor.drain_durable_deliveries_at_safe_point().await);

            let (respond_to, _user_response) = oneshot::channel();
            assert!(
                !actor
                    .queue_input(QueueInputRequest::from_legacy_prompt_id(
                        vec![acp::ContentBlock::Text(acp::TextContent::new(
                            "keep my draft"
                        ))],
                        "human-follow-up".into(),
                        PromptMode::Agent,
                        respond_to,
                    ))
                    .await
            );
            let request = DeliveryRequest {
                session_id: info.id.to_string(),
                delivery_id: "build-result".into(),
                payload: "[Builder]\n## Build Result\ncheck succeeded".into(),
                action: DeliveryAction::Admit,
            };
            assert!(matches!(
                actor.receive_durable_delivery(request.clone()).await,
                DeliveryResult::Admitted { receipt } if receipt.progress == DeliveryProgress::Queued
            ));
            let admitted = notifications(&mut events);
            let queued = queue_snapshots(&admitted);
            let delivery = queued
                .last()
                .unwrap()
                .entries
                .iter()
                .find(|entry| entry.text == request.payload)
                .unwrap();
            assert_eq!(delivery.kind, "delivery");
            let queued_id = delivery.id.clone();

            let gate = actor.tool_context.task_wake_suppressed.as_ref().unwrap();
            gate.set(true);
            assert!(!actor.drain_durable_deliveries_at_safe_point().await);
            assert!(notifications(&mut events).is_empty());
            assert_eq!(actor.state.lock().await.pending_inputs.len(), 2);
            gate.set(false);

            assert!(actor.drain_durable_deliveries_at_safe_point().await);
            let published = notifications(&mut events);
            let interjections: Vec<_> = published
                .iter()
                .filter(|notification| notification.method.as_ref() == "x.ai/session/interjection")
                .map(|notification| {
                    serde_json::from_str::<serde_json::Value>(notification.params.get()).unwrap()
                })
                .collect();
            assert_eq!(interjections.len(), 1);
            assert_eq!(interjections[0].get("text").unwrap(), &request.payload);
            assert_eq!(interjections[0].get("interjectionId").unwrap(), &queued_id);
            let snapshots = queue_snapshots(&published);
            assert_eq!(snapshots.len(), 1);
            assert_eq!(snapshots[0].entries.len(), 1);
            assert_eq!(snapshots[0].entries[0].id, "human-follow-up");
            assert_eq!(snapshots[0].entries[0].kind, "prompt");
            assert_eq!(snapshots[0].entries[0].text, "keep my draft");
            assert_eq!(snapshots[0].running_prompt_id.as_deref(), Some("running"));

            assert!(!actor.drain_durable_deliveries_at_safe_point().await);
            assert!(notifications(&mut events).is_empty());
            let (respond_to, response) = oneshot::channel();
            persistence
                .handle
                .tx
                .send(PersistenceMsg::FlushAndAck { respond_to })
                .unwrap();
            response.await.unwrap().unwrap();
            let restored =
                JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned())
                    .load_session(&info)
                    .await
                    .unwrap();
            let echoes: Vec<_> = restored
                .updates
                .iter()
                .filter_map(|update| match update {
                    SessionUpdate::Acp(notification) => match &notification.update {
                        acp::SessionUpdate::UserMessageChunk(chunk) => match &chunk.content {
                            acp::ContentBlock::Text(text) => Some(text.text.as_str()),
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            assert_eq!(echoes, vec![request.payload.as_str()]);
            assert_eq!(actor.chat_state_handle.get_conversation().await.len(), 1);

            let failed = DeliveryRequest {
                delivery_id: "second-build-result".into(),
                payload: "second result".into(),
                ..request
            };
            assert!(matches!(
                actor.receive_durable_delivery(failed).await,
                DeliveryResult::Admitted { .. }
            ));
            notifications(&mut events);
            persistence_task.abort();
            assert!((&mut persistence_task).await.unwrap_err().is_cancelled());
            assert!(!actor.drain_durable_deliveries_at_safe_point().await);
            assert!(notifications(&mut events).iter().all(|notification| {
                notification.method.as_ref() != "x.ai/session/interjection"
                    && notification.method.as_ref() != "x.ai/queue/changed"
            }));
            assert_eq!(actor.state.lock().await.pending_inputs.len(), 2);
        })
        .await;
}
