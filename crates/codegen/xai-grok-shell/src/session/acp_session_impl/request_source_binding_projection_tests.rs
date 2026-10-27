use super::*;

#[tokio::test(flavor = "current_thread")]
async fn delivery_and_notification_sources_use_distinct_receipt_identities_on_one_prompt() {
    use crate::session::delivery_receiver::{DeliveryAction, DeliveryRequest, DeliveryResult};
    use super::super::super::notification_drain::{NotificationBatchPayload, NotificationBatchRow};
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage);
        let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (gateway, mut events) = mpsc::unbounded_channel();
        let (actor, _) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
            gateway, persistence.handle.tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
        actor.state.lock().await.running_task = Some(super::super::super::support::running_task_stub("same-prompt"));
        let mut expected = Vec::new();
        for id in ["delivery-one", "delivery-two"] {
            let request = DeliveryRequest { session_id: info.id.to_string(), delivery_id: id.into(), payload: "same output".into(), action: DeliveryAction::Admit };
            let digest = request.digest();
            let (respond_to, response) = oneshot::channel();
            persistence.handle.tx.send(PersistenceMsg::ReceiveDelivery { request, respond_to }).unwrap();
            assert!(matches!(response.await.unwrap().unwrap(), DeliveryResult::Admitted { .. }));
            assert!(actor.project_durable_delivery(id, &digest, "same-prompt", "same output".into()).await.unwrap());
            let SourceBindingObservation::Strict { identity: StrictSourceOrigin::Delivery { session_id, delivery_id }, binding } = observed(&mut events).await else { panic!("exact delivery identity"); };
            assert_eq!(session_id, "test-actor");
            assert_eq!(delivery_id, id);
            expected.push(source(&binding));
            assert!(!actor.project_durable_delivery(id, &digest, "same-prompt", "same output".into()).await.unwrap());
            let SourceBindingObservation::Strict { binding, .. } = observed(&mut events).await else { panic!("retained delivery source"); };
            assert_eq!(source(&binding), *expected.last().unwrap());
        }
        assert_ne!(expected[0], expected[1]);
        let payload = NotificationBatchPayload::Rows { id: "actual-monitor-batch".into(), rows: vec![NotificationBatchRow::MonitorEvent { task_id: "actual-monitor".into(), text: "same output".into() }] };
        assert!(actor.project_notification_batch(Vec::new(), "same-prompt", "same output".into(), &payload).await.unwrap());
        let SourceBindingObservation::Strict { identity: StrictSourceOrigin::Notification { session_id, batch_id, completions }, binding } = observed(&mut events).await else { panic!("exact batch identity"); };
        assert_eq!(session_id, "test-actor");
        assert_eq!(batch_id, "actual-monitor-batch");
        assert!(completions.is_empty());
        expected.push(source(&binding));
        assert!(!actor.project_notification_batch(Vec::new(), "same-prompt", "retry body is not identity".into(), &payload).await.unwrap());
        let SourceBindingObservation::Strict { binding, .. } = observed(&mut events).await else { panic!("retained batch source"); };
        assert_eq!(source(&binding), *expected.last().unwrap());
        flush(&persistence.handle.tx).await;
        let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned());
        let restored = reopened.load_session(&info).await.unwrap();
        assert_eq!(restored.chat_history.iter().map(ConversationItem::source).collect::<Vec<_>>(), expected);
        assert_eq!(actor.chat_state_handle.get_conversation().await.len(), 3);
        assert!(restored.updates.iter().all(|update| !matches!(update, SessionUpdate::Acp(_))));
        let event = completion("observed-tool-child");
        reopened.prepare_subagent_completion(&info, event.clone()).await.unwrap();
        *actor.current_prompt_id.lock().unwrap() = Some("same-prompt".into());
        actor.chat_state_handle.push_assistant_response_and_ack(ConversationItem::assistant_tool_calls(vec![xai_grok_sampling_types::ToolCall {
            id: "actual-read-call".into(), name: "read_file".into(), arguments: "{}".into(),
        }])).await.unwrap();
        actor.commit_observed_subagent_tool_result(ConversationItem::tool_result("actual-read-call", event.text.clone()), vec![
            xai_grok_tools::types::completion_observation::ObservedCompletion::Subagent(xai_grok_tools::types::completion_observation::ObservedSubagentCompletion {
                subagent_id: event.id.subagent_id.clone(), attempt_id: event.id.attempt_id.clone(), text: event.text.clone(),
            }),
        ]).await.unwrap();
        let SourceBindingObservation::Strict { identity: StrictSourceOrigin::CompletionToolResult { session_id, completions, tool_call_id }, binding } = observed(&mut events).await else { panic!("actual tool-call and completion identity"); };
        assert_eq!(session_id, "test-actor");
        assert_eq!(tool_call_id, "actual-read-call");
        assert_eq!(completions, vec![event.id.into()]);
        flush(&persistence.handle.tx).await;
        let persisted = reopened.load_session(&info).await.unwrap();
        let result = persisted.chat_history.iter().find(|item| matches!(item, ConversationItem::ToolResult(result) if result.tool_call_id == "actual-read-call")).unwrap();
        assert_eq!(source(&binding), result.source());
        assert_eq!(persisted.chat_history.len(), 5);
    }).await;
}
