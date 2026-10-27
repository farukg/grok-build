use super::*;

#[tokio::test(flavor = "current_thread")]
async fn lost_strict_reply_after_real_commit_is_unknown_until_receipt_reconciliation() {
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let event = completion("lost-reply-child");
        storage.prepare_subagent_completion(&info, event.clone()).await.unwrap();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
        let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (proxy, mut requests) = mpsc::unbounded_channel();
        let real = persistence.handle.tx.clone();
        let _proxy = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(async move {
            let mut losses_remaining = 1;
            while let Some(request) = requests.recv().await {
                match request {
                    PersistenceMsg::AppendInputAndAck { projection, admission, respond_to } if losses_remaining > 0 => {
                        losses_remaining -= 1;
                        let (committed, response) = oneshot::channel();
                        real.send(PersistenceMsg::AppendInputAndAck { projection, admission, respond_to: committed }).unwrap();
                        assert!(matches!(response.await.unwrap().unwrap(), xai_chat_state::CompletionProjectionAck::Appended));
                        drop(respond_to);
                    }
                    request => { real.send(request).unwrap(); }
                }
            }
        }));
        let (gateway, mut events) = mpsc::unbounded_channel();
        let (actor, _) = super::super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
            gateway, proxy.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(proxy.clone()))).await;
        actor.state.lock().await.running_task = Some(super::super::super::support::running_task_stub("lost-reply-prompt"));
        let projection = CompletionProjection::new(event.id.clone(), "lost-reply-prompt".into(), event.text.clone());
        assert!(actor.commit_subagent_projection(projection.clone(), "lost-reply-prompt").await.is_err());
        let observation = observed(&mut events).await;
        let SourceBindingObservation::Strict { binding: InputSourceBinding::Unknown { cause: InputSourceUnknown::ReplyLost }, .. } = &observation else { panic!("a committed but lost reply is not an observed source"); };
        assert!(actor.chat_state_handle.get_conversation().await.is_empty());
        flush(&proxy).await;
        let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned());
        let restored = reopened.load_session(&info).await.unwrap();
        assert_eq!(restored.chat_history.len(), 1, "reply loss does not roll back the real durable append");
        let persisted_source = restored.chat_history[0].source();
        assert!(matches!(persisted_source, xai_grok_sampling_types::SourceOccurrence::Admitted { .. }));
        assert!(restored.updates.iter().all(|update| !matches!(update, SessionUpdate::Acp(_))));
        assert!(restored.updates.iter().any(|update| matches!(update, SessionUpdate::Xai(notification) if matches!(&notification.update, XaiSessionUpdate::RequestSourceObserved { observation: replay } if replay == &observation))));
        assert!(!actor.commit_subagent_projection(projection, "lost-reply-prompt").await.unwrap());
        let SourceBindingObservation::Strict { binding, .. } = observed(&mut events).await else { panic!("retained receipt source"); };
        assert_eq!(source(&binding), persisted_source);
        assert_eq!(actor.chat_state_handle.get_conversation().await.len(), 1);
        flush(&proxy).await;
        assert_eq!(reopened.load_session(&info).await.unwrap().chat_history.len(), 1);
    }).await;
}
