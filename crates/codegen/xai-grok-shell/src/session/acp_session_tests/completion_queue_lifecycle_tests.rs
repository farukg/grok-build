use super::*;
use crate::session::completion_delivery::{
    CompletionArtifacts, CompletionEventPolicy, CompletionObligation, SubagentCompletionEvent,
};

async fn shutdown_and_recover_child() {
    let CompletionWakeReceiverFixture {
        actor,
        sender,
        task,
        writes: _writes,
    } = completion_wake_receiver_fixture().await;
    let completion = test_completion_wake("shutdown-child");
    let event = SubagentCompletionEvent {
        id: completion.id.clone(),
        state: xai_grok_tools::implementations::grok_build::task::types::SubagentState::Failed {
            message: "child upload denied during shutdown".into(),
        },
        text: "shutdown-child: child upload denied during shutdown".into(),
        output: "child upload denied during shutdown".into(),
        child_session_id: "shutdown-child-session".into(),
        tool_calls: 2,
        turns: 1,
        duration_ms: 100,
        tokens_used: 11,
        completed_at: chrono::Utc::now(),
        policy: CompletionEventPolicy::Automatic,
        artifacts: CompletionArtifacts::None,
    };
    let (respond_to, receive) = oneshot::channel();
    actor
        .notifications
        .persistence_tx
        .send(PersistenceMsg::PrepareSubagentCompletion {
            event: event.clone(),
            respond_to,
        })
        .unwrap();
    let committed = receive
        .await
        .expect("producer acknowledgement")
        .expect("actual producer commit");
    assert_eq!(committed.event.completed_at, event.completed_at);
    assert_eq!(committed.event.state, event.state);
    let (respond_to, receive) = oneshot::channel();
    sender
        .send(SessionCommand::PublishSubagentCompletion {
            event: committed.event,
            will_wake: true,
            respond_to,
        })
        .unwrap();
    let publication = receive
        .await
        .expect("publication acknowledgement")
        .expect("real parent publication");
    let (respond_to, wake_receipt) = oneshot::channel();
    sender
        .send(SessionCommand::SubagentCompletionWake {
            prompt_id: "shutdown-child-wake".into(),
            completion: completion.clone(),
            text: publication.text.clone(),
            respond_to,
        })
        .unwrap();
    let (respond_to, receive) = oneshot::channel();
    sender
        .send(SessionCommand::GetToolOverrides { respond_to })
        .unwrap();
    receive
        .await
        .expect("parent FIFO includes the real child wake");
    assert!(actor.state.lock().await.pending_inputs.iter().any(|item| {
        matches!(item.input_origin.as_prompt_origin(),
            crate::session::PromptOrigin::SubagentCompleted { completion: queued }
                if queued.id == completion.id)
    }));
    assert!(!completion_projected(&actor, "shutdown-child").await);

    sender
        .send(SessionCommand::Shutdown(
            crate::session::ShutdownKind::CancelRunningTurn,
        ))
        .unwrap();
    task.await.expect("actual session shutdown completes");
    assert_eq!(
        wake_receipt
            .await
            .expect("shutdown settles the queued wake")
            .unwrap()
            .stop_reason,
        acp::StopReason::Cancelled
    );
    assert!(actor.state.lock().await.pending_inputs.is_empty());
    assert!(!completion_projected(&actor, "shutdown-child").await);
    let (respond_to, receive) = oneshot::channel();
    actor
        .notifications
        .persistence_tx
        .send(PersistenceMsg::RecoverSubagentCompletions { respond_to })
        .unwrap();
    let recovered = receive
        .await
        .expect("recovery acknowledgement")
        .expect("real persisted recovery");
    assert!(matches!(
        recovered.completion_delivery.wake_policy,
        crate::session::completion_delivery::CompletionWakePolicy::Automatic
    ));
    assert!(
        recovered.completion_delivery.receipts.is_empty(),
        "shutdown did not attest model consumption"
    );
    assert!(
        matches!(recovered.completion_delivery.obligations.as_slice(),
        [CompletionObligation::Pending { id, payload }]
            if id == &completion.id && payload.text == publication.text)
    );
    assert!(
        publication
            .text
            .contains("child upload denied during shutdown")
    );
    assert!(matches!(publication.notification.update,
        crate::extensions::notification::SessionUpdate::SubagentFinished {
            child_session_id, status, error, ..
        } if child_session_id == "shutdown-child-session"
            && status == "failed"
            && error.as_deref() == Some("child upload denied during shutdown")));
}

#[tokio::test(flavor = "current_thread")]
async fn human_prompt_command_promotes_parked_child_without_manual_clear_or_kick() {
    tokio::task::LocalSet::new().run_until(tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async {
            let CompletionWakeReceiverFixture { actor, sender, task: _task, writes: _writes } = completion_wake_receiver_fixture().await;
            actor.state.lock().await.running_task = None;
            actor.set_completion_wake_policy(crate::session::completion_delivery::CompletionWakePolicy::HumanStopped);
            actor.state.lock().await.notifications_suppressed = true;
            let completion = test_completion_wake("human-resume-child");
            let prompt_id = completion_wake_prompt_id(&completion);
            let (respond_to, _response) = oneshot::channel();
            sender.send(SessionCommand::SubagentCompletionWake {
                prompt_id: prompt_id.clone(), completion,
                text: "human-resume-child finished".to_string(), respond_to,
            }).unwrap();
            let (respond_to, receive) = oneshot::channel();
            sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
            receive.await.unwrap();
            assert!(actor.state.lock().await.running_task.is_none());
            let (respond_to, _response) = oneshot::channel();
            sender.send(SessionCommand::Prompt {
                prompt_id: "human-reengagement".to_string(),
                prompt_blocks: Vec::new(), prompt_mode: PromptMode::Agent,
                artifact_upload_ctx: None, client_identifier: None, screen_mode: None,
                verbatim: false, traceparent: None, json_schema: None, send_now: false,
                admission: None, tool_overrides_update: None, respond_to,
                prompt_admitted: None, persist_ack: None, parsed_prompt_tx: None,
            }).unwrap();
            let (respond_to, receive) = oneshot::channel();
            sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
            receive.await.unwrap();
            let state = actor.state.lock().await;
            assert_eq!(state.running_prompt_id(), Some(prompt_id.as_str()));
            assert!(!state.notifications_suppressed);
            assert!(state.pending_inputs.iter().any(|input| input.prompt_id == "human-reengagement"));
        },
    )).await.expect("the actual actor must promote the parked completion on human intake");
}

#[tokio::test(flavor = "current_thread")]
async fn restore_command_rewrites_human_stopped_even_on_a_fresh_clear_actor() {
    tokio::task::LocalSet::new().run_until(tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async {
            let CompletionWakeReceiverFixture { actor, sender, task: _task, writes: _writes } = completion_wake_receiver_fixture().await;
            actor.notifications.persistence_tx.send(PersistenceMsg::CompletionWakePolicy(
                crate::session::completion_delivery::CompletionWakePolicy::HumanStopped,
            )).unwrap();
            assert_eq!(actor.persisted_completion_state().await.unwrap().wake_policy,
                crate::session::completion_delivery::CompletionWakePolicy::HumanStopped);
            sender.send(SessionCommand::RestoreCompletionWakePolicy(
                crate::session::completion_delivery::CompletionWakePolicy::HumanStopped,
            )).unwrap();
            let (respond_to, receive) = oneshot::channel();
            sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
            receive.await.unwrap();
            assert_eq!(actor.persisted_completion_state().await.unwrap().wake_policy,
                crate::session::completion_delivery::CompletionWakePolicy::Automatic);
            assert!(!actor.state.lock().await.notifications_suppressed);
        },
    )).await.expect("restart must durably retire the historical stop barrier");
}

#[tokio::test(flavor = "current_thread")]
async fn restore_command_promotes_a_previously_parked_completion() {
    tokio::task::LocalSet::new().run_until(tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async {
            let CompletionWakeReceiverFixture { actor, sender, task: _task, writes: _writes } = completion_wake_receiver_fixture().await;
            actor.state.lock().await.running_task = None;
            actor.set_completion_wake_policy(crate::session::completion_delivery::CompletionWakePolicy::HumanStopped);
            actor.state.lock().await.notifications_suppressed = true;
            let completion = test_completion_wake("restart-parked");
            let prompt_id = completion_wake_prompt_id(&completion);
            let (respond_to, _response) = oneshot::channel();
            sender.send(SessionCommand::SubagentCompletionWake {
                prompt_id: prompt_id.clone(), completion,
                text: "restart-parked finished".to_string(), respond_to,
            }).unwrap();
            let (respond_to, receive) = oneshot::channel();
            sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
            receive.await.unwrap();
            assert!(actor.state.lock().await.running_task.is_none());
            sender.send(SessionCommand::RestoreCompletionWakePolicy(
                crate::session::completion_delivery::CompletionWakePolicy::HumanStopped,
            )).unwrap();
            let (respond_to, receive) = oneshot::channel();
            sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
            receive.await.unwrap();
            assert_eq!(actor.state.lock().await.running_prompt_id(), Some(prompt_id.as_str()));
        },
    )).await.expect("restart must re-admit the parked completion without a later prompt");
}

#[tokio::test(flavor = "current_thread")]
async fn terminal_notifications_keep_their_identity_during_an_active_turn() {
    tokio::task::LocalSet::new().run_until(tokio::time::timeout(
        std::time::Duration::from_secs(5),
        async {
            let CompletionWakeReceiverFixture { actor, sender, task: _task, writes: _writes } = completion_wake_receiver_fixture().await;
            actor.session_turn_active.store(true, std::sync::atomic::Ordering::SeqCst);
            for index in 0..60 {
                sender.send(SessionCommand::InjectNotification {
                    prompt_id: format!("terminal-{index}"), prompt_blocks: Vec::new(),
                    priority: NotificationPriority::Next,
                    source: NotificationSource::MonitorCompleted { task_id: format!("monitor-{index}") },
                }).unwrap();
            }
            let (respond_to, receive) = oneshot::channel();
            sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
            receive.await.unwrap();
            let state = actor.state.lock().await;
            assert_eq!(state.pending_notifications.len(), 60);
            assert!(state.pending_notifications.iter().all(|notification|
                matches!(notification.source, NotificationSource::MonitorCompleted { .. })));
        },
    )).await.expect("terminal obligations must bypass the lossy transient-event buffer");
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_preserves_unprojected_child_for_real_persistence_recovery() {
    tokio::task::LocalSet::new()
        .run_until(tokio::time::timeout(
            std::time::Duration::from_secs(5),
            shutdown_and_recover_child(),
        ))
        .await
        .expect("shutdown and durable child recovery must finish within the protocol budget");
}
