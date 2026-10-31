use super::*;
use crate::session::completion_delivery::{
    CompletionAdmission, CompletionArtifacts, CompletionEventPolicy, CompletionObligation,
    CompletionOffer, CompletionSuppression, CompletionWakePolicy, SuppressedCompletionSource,
};
use xai_grok_tools::implementations::grok_build::task::types::{
    ParentSessionRevocation, SubagentEvent, SubagentState,
};

#[tokio::test(flavor = "current_thread")]
async fn acknowledged_stop_seals_registered_delayed_prepare_across_new_human_authority() {
    #[derive(Clone, Copy)]
    enum StopAck { Forward, LoseOnce }
    tokio::task::LocalSet::new().run_until(async {
      for ack in [StopAck::Forward, StopAck::LoseOnce] {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::info::Info { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let event = crate::session::SubagentCompletionEvent {
            id: xai_chat_state::SubagentCompletionId {
                subagent_id: "registered-delayed-child".into(),
                attempt_id: xai_message_delivery_core::AttemptId::mint(1),
            },
            state: SubagentState::Completed,
            text: "original successful result".into(), output: "original output".into(),
            child_session_id: "real-child-session".into(), tool_calls: 2, turns: 1,
            duration_ms: 100, tokens_used: 7, completed_at: chrono::Utc::now(),
            policy: CompletionEventPolicy::Automatic, artifacts: CompletionArtifacts::None,
        };
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
        let _owner = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (gateway, _notifications) = mpsc::unbounded_channel();
        let (proxy, mut mutations) = mpsc::unbounded_channel();
        let durable = persistence.handle.tx.clone();
        let _proxy_owner = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(async move {
            let mut fault = ack;
            while let Some(mutation) = mutations.recv().await {
                match mutation {
                    PersistenceMsg::SealHumanStoppedCompletions { ids, workflow_episodes, respond_to } => {
                        let (committed, receive) = oneshot::channel();
                        durable.send(PersistenceMsg::SealHumanStoppedCompletions { ids, workflow_episodes, respond_to: committed }).unwrap();
                        let result = receive.await.unwrap();
                        match fault {
                            StopAck::LoseOnce => { assert!(result.is_ok()); drop(respond_to); fault = StopAck::Forward; }
                            StopAck::Forward => { let _ = respond_to.send(result); }
                        }
                    }
                    other => { durable.send(other).unwrap(); }
                }
            }
        }));
        let mut actor = super::super::super::super::support::create_test_actor(0, 256_000, 85, gateway, proxy).await;
        let (children, mut requests) = mpsc::unbounded_channel();
        actor.tool_context.subagent_event_tx = Some(children);
        let registered = event.id.clone();
        let parent = info.id.to_string();
        let revocations = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(async move {
            while let Some(request) = requests.recv().await {
                let SubagentEvent::RevokeParentSession { parent_session_id, respond_to, .. } = request else {
                    panic!("unexpected producer request");
                };
                assert_eq!(parent_session_id, parent);
                let _ = respond_to.send(Ok(ParentSessionRevocation {
                    parent_session_id, completions: vec![registered.clone()],
                }));
            }
        }));
        let stopped = bounded(actor.cancel_running_task(crate::session::CancelOptions {
            trigger: Some(crate::session::CancelTrigger::CtrlC), user_initiated: true, ..Default::default()
        })).await;
        match ack {
            StopAck::Forward => {
                assert!(stopped.settled);
                assert!(matches!(stopped.policy, crate::session::acp_session::cancel::StopPolicyCommit::Committed));
            }
            StopAck::LoseOnce => {
                assert!(!stopped.settled);
                let crate::session::acp_session::cancel::StopPolicyCommit::Failed(error) = stopped.policy else {
                    panic!("lost durable ACK must remain unconfirmed");
                };
                assert_eq!(error.to_string(), "Stop suppression commit acknowledgement lost");
                assert!(actor.state.lock().await.notifications_suppressed);
                assert!(actor.tool_context.task_wake_suppressed.as_ref().unwrap().get());
            }
        }
        let sealed = actor.persisted_completion_state().await.unwrap();
        assert!(sealed.obligations.iter().any(|obligation| matches!(obligation,
            CompletionObligation::Suppressed { id, reason: CompletionSuppression::HumanStopped,
                source: SuppressedCompletionSource::AwaitingEnrollment } if id == &event.id)));
        assert_eq!(sealed.obligations.iter().filter(|obligation| obligation.id() == event.id).count(), 1);
        bounded(actor.clear_wake_suppression(crate::session::acp_session::cancel::WakeReengagement::NewHumanMessage)).await.unwrap();
        let reopened = actor.persisted_completion_state().await.unwrap();
        assert_eq!(reopened.wake_policy, CompletionWakePolicy::Automatic);
        assert_eq!(reopened.obligations.iter().filter(|obligation| obligation.id() == event.id).count(), 1);
        storage.prepare_subagent_completion(&info, event.clone()).await.unwrap();
        let prepared = actor.persisted_completion_state().await.unwrap();
        assert!(prepared.obligations.iter().any(|obligation| matches!(obligation,
            CompletionObligation::Preparing { event: retained,
                admission: CompletionAdmission::Suppressed(CompletionSuppression::HumanStopped) }
                if retained.id == event.id && retained.output == event.output && retained.state == event.state)));
        let projection = xai_chat_state::CompletionProjection::new(event.id.clone(), "forbidden-old-wake".into(), event.text.clone());
        let admission = xai_chat_state::InputAdmission {
            parent_task: xai_chat_state::ParentTask::Unknown,
            cancellation: tokio_util::sync::CancellationToken::new(),
        };
        let denied = storage.append_completion_commit_aware(&info, &projection, &admission).await;
        assert!(matches!(denied, Err(xai_chat_state::StrictAppendError::NotCommitted(error))
            if error.kind() == std::io::ErrorKind::PermissionDenied));
        let published = storage.publish_subagent_completion(&info, &event, true).await.unwrap();
        assert!(matches!(published.offer, CompletionOffer::None));
        assert!(published.text.contains(&event.text));
        let retained = storage.recover_subagent_completions(&info).await.unwrap();
        assert!(retained.completion_delivery.receipts.is_empty());
        assert!(retained.completion_delivery.obligations.iter().any(|obligation| matches!(obligation,
            CompletionObligation::Suppressed { id, reason: CompletionSuppression::HumanStopped,
                source: SuppressedCompletionSource::Enrolled(payload) }
                if id == &event.id && payload.text == event.text)));
        let mut fresh = event.clone();
        fresh.id.attempt_id = xai_message_delivery_core::AttemptId::mint(2);
        fresh.output = "new authorized result".into();
        fresh.text = "new authorized result".into();
        assert_ne!(fresh.id, event.id);
        storage.prepare_subagent_completion(&info, fresh.clone()).await.unwrap();
        let published = storage.publish_subagent_completion(&info, &fresh, true).await.unwrap();
        assert!(!matches!(published.offer, CompletionOffer::None));
        let projection = xai_chat_state::CompletionProjection::new(fresh.id, "fresh-authorized-wake".into(), fresh.text);
        assert!(matches!(storage.append_completion_commit_aware(&info, &projection, &admission).await.unwrap(),
            xai_chat_state::CompletionProjectionAck::Appended));
        assert!(actor.state.lock().await.running_task.is_none());
        drop(revocations);
      }
    }).await;
}
