use super::*;
use crate::extensions::sampling_lifecycle::{SamplingLifecycle, SamplingOutcome, SamplingPhase};
use crate::session::acp_session::replay_buffer_send_update_tests::make_replay_send_update_fixture;
use xai_grok_sampler::{SamplingErrorKind, SamplingEvent};
use xai_grok_sampling_types::SamplingError;

fn retry(request_id: &RequestId, next_attempt: u64) -> SamplingEvent {
    SamplingEvent::Retrying {
        request_id: request_id.clone(), attempt: 1, max_retries: 4, backoff_ms: 1200_000,
        deadline: RetryDeadline::after(std::time::Duration::from_secs(1200)).unwrap(),
        next_attempt: RequestAttemptId(next_attempt), kind: SamplingErrorKind::Http,
        reason: "transport retry".into(), doom_loop_triggers: None, doom_loop_aborted_at_chunk: None,
    }
}

fn delta(request_id: &RequestId, bytes: &str) -> SamplingEvent {
    SamplingEvent::ToolCallDelta {
        request_id: request_id.clone(), tool_index: 0, id: None, name: Some("edit_file".into()),
        arguments_delta: Some(bytes.into()),
    }
}

fn take_notification(rx: &mut tokio::sync::mpsc::UnboundedReceiver<SessionEvent>) -> SessionNotification {
    let SessionEvent::Notification(notification) = rx.try_recv().expect("notification queued") else {
        panic!("expected notification");
    };
    notification
}

fn lifecycle(notification: &SessionNotification) -> SamplingLifecycle {
    let SessionNotification::Xai(notification) = notification else { panic!("expected lifecycle"); };
    serde_json::from_value(notification.meta.as_ref().unwrap()[SAMPLING_LIFECYCLE_META_KEY].clone()).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn retry_retires_old_bytes_before_any_new_delta_and_keeps_background_output() {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        let actor = Arc::new(fixture.actor);
        *actor.current_prompt_id.lock().unwrap() = Some("prompt".into());
        let request = RequestId::from("request-A");
        actor.register_sampling_request(&request);
        actor.turn_stream_drained.lock().insert(request.clone(), StreamOwnership::with_waiter(None));
        actor.handle_sampling_event(delta(&request, "old-attempt")).await;
        let stale = take_notification(&mut fixture.event_rx);
        actor.handle_sampling_event(retry(&request, 2)).await;
        let scheduled = take_notification(&mut fixture.event_rx);
        let owner = lifecycle(&scheduled).owner;
        assert_eq!(owner.request_id, request);
        assert_eq!(owner.attempt, RequestAttemptId(2));
        actor.emit_buffered(scheduled).await;
        actor.emit_buffered(stale).await;
        // This is the real native dispatch evidence, not a content or StreamStarted surrogate.
        actor.handle_sampling_event(SamplingEvent::LocalRequest {
            request_id: request.clone(), attempt: RequestAttemptId(2),
            evidence: xai_grok_sampler::request_capture::LocalRequestEvidence::DispatchStarted { started_at: Default::default() },
        }).await;
        let started = take_notification(&mut fixture.event_rx);
        assert_eq!(lifecycle(&started).owner, owner);
        assert!(matches!(lifecycle(&started).phase, SamplingPhase::DispatchStarted { .. }));
        actor.emit_buffered(started).await;
        let background = RequestId::from("background-B");
        actor.register_sampling_request(&background);
        actor.turn_stream_drained.lock().insert(background.clone(), StreamOwnership::with_waiter(None));
        actor.handle_sampling_event(delta(&background, "valid-background")).await;
        actor.emit_buffered(take_notification(&mut fixture.event_rx)).await;
        for _ in 0..8 { tokio::task::yield_now().await; }
        let sent = fixture.sent_ext.lock().await;
        let notifications: Vec<crate::extensions::notification::SessionNotification> = sent.iter()
            .filter(|notification| notification.method.as_ref() == "x.ai/session_notification")
            .map(|notification| serde_json::from_str(notification.params.get()).unwrap()).collect();
        assert_eq!(notifications.len(), 3, "retry, actual dispatch, and legitimate background delta only");
        assert!(matches!(notifications[0].update, XaiSessionUpdate::RetryState(_)));
        assert!(matches!(notifications[1].update, XaiSessionUpdate::SamplingLifecycle { .. }));
        let XaiSessionUpdate::ToolCallDeltaChunk { arguments_delta, .. } = &notifications[2].update else { panic!("background must survive"); };
        assert_eq!(arguments_delta.as_deref(), Some("valid-background"));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_request_low_display_count_and_old_terminal_keep_separate_owners() {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        let actor = Arc::new(fixture.actor);
        *actor.current_prompt_id.lock().unwrap() = Some("prompt".into());
        let first = RequestId::from("request-A");
        let next = RequestId::from("request-B");
        let deadline = RetryDeadline::after(std::time::Duration::from_secs(3)).unwrap();
        for (request, attempt) in [(&first, 3), (&next, 1)] {
            actor.schedule_sampling_retry(request, &deadline, crate::extensions::notification::RetryState::Retrying {
                attempt, max_retries: 5, reason: "resubmit".into(), error_type: None,
                next_attempt_at_ms: deadline.projection().wall_deadline_ms,
            });
        }
        let first_retry = take_notification(&mut fixture.event_rx);
        let next_retry = take_notification(&mut fixture.event_rx);
        assert_eq!(lifecycle(&first_retry).owner.request_id, first);
        assert_eq!(lifecycle(&next_retry).owner.request_id, next);
        let SessionNotification::Xai(next_notification) = &next_retry else { panic!("expected retry"); };
        assert!(matches!(next_notification.update, XaiSessionUpdate::RetryState(crate::extensions::notification::RetryState::Retrying { attempt: 1, .. })));
        actor.turn_stream_drained.lock().insert(first.clone(), StreamOwnership::with_waiter(None));
        actor.handle_sampling_event(SamplingEvent::Failed {
            request_id: first.clone(), error: (&SamplingError::EventStreamError("old request failed".into())).into(),
        }).await;
        let terminal = take_notification(&mut fixture.event_rx);
        assert_eq!(lifecycle(&terminal).owner.request_id, first);
        assert_eq!(lifecycle(&terminal).phase, SamplingPhase::Finished { outcome: SamplingOutcome::Failed });
        assert!(actor.accepts_sampling_owner(&lifecycle(&next_retry).owner));
        actor.cancel_active_sampling_requests();
        let cancel = take_notification(&mut fixture.event_rx);
        assert_eq!(lifecycle(&cancel).owner.request_id, next);
        assert_eq!(lifecycle(&cancel).phase, SamplingPhase::Finished { outcome: SamplingOutcome::Cancelled });
    }).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn actor_sender_delay_does_not_restart_the_shell_scheduler() {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        let actor = Arc::new(fixture.actor);
        *actor.current_prompt_id.lock().unwrap() = Some("prompt".into());
        let request = RequestId::random();
        let started = tokio::time::Instant::now();
        let deadline = RetryDeadline::after(std::time::Duration::from_secs(1200)).unwrap();
        actor.schedule_sampling_retry(&request, &deadline, crate::extensions::notification::RetryState::Retrying {
            attempt: 1, max_retries: 5, reason: "capacity".into(), error_type: None,
            next_attempt_at_ms: deadline.projection().wall_deadline_ms,
        });
        tokio::time::advance(std::time::Duration::from_secs(90)).await;
        let queued = take_notification(&mut fixture.event_rx);
        let SamplingPhase::RetryScheduled { deadline: projection } = lifecycle(&queued).phase else { panic!("expected scheduler state"); };
        let xai_grok_sampler::retry_deadline::RetryClockMapping::Correlated { earliest_due_at, latest_due_at } = projection.map_local() else { panic!("same process clock"); };
        let original_due_at = started + std::time::Duration::from_secs(1200);
        assert!(earliest_due_at <= original_due_at && original_due_at <= latest_due_at);
        actor.emit_buffered(queued).await;
        deadline.wait().await.unwrap();
        assert_eq!(tokio::time::Instant::now(), original_due_at);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn terminal_removal_and_late_dispatch_cannot_revive_failed_bytes() {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        let actor = Arc::new(fixture.actor);
        *actor.current_prompt_id.lock().unwrap() = Some("original-prompt".into());
        let request = RequestId::from("terminal-request");
        actor.register_sampling_request(&request);
        actor.turn_stream_drained.lock().insert(request.clone(), StreamOwnership::with_waiter(None));
        actor.handle_sampling_event(delta(&request, "failed-bytes")).await;
        let stale = take_notification(&mut fixture.event_rx);
        *actor.current_prompt_id.lock().unwrap() = Some("different-prompt".into());
        actor.handle_sampling_event(SamplingEvent::Failed {
            request_id: request.clone(), error: (&SamplingError::EventStreamError("failure".into())).into(),
        }).await;
        let terminal = take_notification(&mut fixture.event_rx);
        assert_eq!(lifecycle(&terminal).owner.prompt_id, "original-prompt");
        assert!(!actor.turn_stream_drained.lock().contains_key(&request));
        assert!(!actor.accepts_sampling_notification(&stale));
        actor.emit_buffered(terminal).await;
        actor.emit_buffered(stale).await;
        actor.handle_sampling_event(SamplingEvent::LocalRequest {
            request_id: request, attempt: RequestAttemptId(1),
            evidence: xai_grok_sampler::request_capture::LocalRequestEvidence::DispatchStarted { started_at: Default::default() },
        }).await;
        assert!(fixture.event_rx.try_recv().is_err(), "a terminal owner cannot dispatch again");
        for _ in 0..8 { tokio::task::yield_now().await; }
        assert_eq!(fixture.sent_ext.lock().await.len(), 1, "only the real terminal is sent");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn actual_transport_doom_producer_survives_delayed_actor_delivery() {
    use xai_grok_sampler::{ApiBackend, RetryPolicy, SamplerActor, SamplerConfig};
    use xai_grok_test_support::{MockInferenceServer, MockModelEntry, ScriptedResponse, sse};
    tokio::task::LocalSet::new().run_until(async {
        let server = MockInferenceServer::start_with_models(vec![MockModelEntry::new("test-model")]).await.unwrap();
        server.enqueue_response("/v1/responses", ScriptedResponse::text(503, "temporary failure"));
        server.enqueue_response("/v1/responses", ScriptedResponse::sse(sse::responses_api_doom_loop_terminal_only_events(
            &["tail_repetition:8@thinking"], "discarded thought", "discarded text", "test-model",
        )));
        server.enqueue_response("/v1/responses", ScriptedResponse::sse(sse::responses_api_reasoning_and_text_events(
            "accepted thought", "accepted text", "test-model",
        )));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = SamplerActor::spawn(SamplerConfig {
            base_url: server.url(), model: "test-model".into(), api_backend: ApiBackend::Responses,
            max_retries: Some(3), doom_loop_recovery: Some(xai_grok_sampling_types::DoomLoopRecoveryPolicy::default()),
            ..Default::default()
        }, RetryPolicy::default(), tx);
        let mut fixture = make_replay_send_update_fixture().await;
        let actor = Arc::new(fixture.actor);
        *actor.current_prompt_id.lock().unwrap() = Some("producer-prompt".into());
        let request = RequestId::from("producer-request");
        actor.register_sampling_request(&request);
        actor.turn_stream_drained.lock().insert(request.clone(), StreamOwnership::with_waiter(None));
        handle.submit(request.clone(), ConversationRequest {
            items: vec![ConversationItem::user("hi")], ..Default::default()
        });
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let event = rx.recv().await.expect("sampler event");
                let terminal = matches!(&event, SamplingEvent::Completed { .. } | SamplingEvent::Failed { .. });
                actor.handle_sampling_event(event).await;
                if terminal { break; }
            }
        }).await.unwrap();
        let mut buffer = crate::agent::update_chunk_merge::ReplayBuffer::new(Some(
            crate::agent::update_chunk_merge::BufferingSettings { max_items: 100, max_bytes: 100_000, max_duration_ms: 10 },
        ));
        while let Ok(SessionEvent::Notification(notification)) = fixture.event_rx.try_recv() {
            if let Some((first, second)) = buffer.consume_chunk(notification) {
                actor.emit_buffered(first).await;
                if let Some(second) = second { actor.emit_buffered(second).await; }
            }
        }
        if let Some(last) = buffer.flush() { actor.emit_buffered(last).await; }
        for _ in 0..8 { tokio::task::yield_now().await; }
        let sent = fixture.sent_ext.lock().await;
        let notifications: Vec<crate::extensions::notification::SessionNotification> = sent.iter()
            .filter(|notification| notification.method.as_ref() == "x.ai/session_notification")
            .map(|notification| serde_json::from_str(notification.params.get()).unwrap()).collect();
        let retries: Vec<_> = notifications.iter().filter_map(|notification| match &notification.update {
            XaiSessionUpdate::RetryState(crate::extensions::notification::RetryState::Retrying { attempt, .. }) => {
                let lifecycle: SamplingLifecycle = serde_json::from_value(notification.meta.as_ref().unwrap()[SAMPLING_LIFECYCLE_META_KEY].clone()).unwrap();
                assert_eq!(lifecycle.owner.request_id, request);
                Some((*attempt, lifecycle.owner.attempt))
            }
            _ => None,
        }).collect();
        assert_eq!(retries, [(1, RequestAttemptId(2)), (1, RequestAttemptId(3))]);
        let dispatches: Vec<_> = notifications.iter().filter_map(|notification| match &notification.update {
            XaiSessionUpdate::SamplingLifecycle { lifecycle } if matches!(lifecycle.phase, SamplingPhase::DispatchStarted { .. }) => Some(lifecycle.owner.attempt),
            _ => None,
        }).collect();
        assert_eq!(dispatches, [RequestAttemptId(1), RequestAttemptId(2), RequestAttemptId(3)]);
        let sent = fixture.sent.lock().await;
        let text = sent.iter().filter_map(|notification| match &notification.update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                acp::ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            },
            _ => None,
        }).collect::<String>();
        assert_eq!(text, "accepted text", "buffered bytes of the failed attempt never reach live output");
        assert_eq!(server.request_count(), 3);
    }).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn auth_early_release_is_followed_by_real_dispatch_of_the_reserved_owner() {
    use xai_grok_login::backend::{ActiveAuthBackend, AuthBackend};
    use xai_grok_sampler::{ApiBackend, RetryPolicy, SamplerActor, SamplerConfig};
    use xai_grok_test_support::{MockInferenceServer, MockModelEntry};
    tokio::task::LocalSet::new().run_until(async {
        let server = MockInferenceServer::start_with_models(vec![MockModelEntry::new("test-model")]).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let manager = xai_grok_login::AuthManager::new(dir.path(), xai_grok_login::GrokComConfig::default());
        manager.hot_swap(xai_grok_login::GrokAuth {
            auth_mode: xai_grok_login::AuthMode::Oidc,
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            ..xai_grok_login::GrokAuth::test_default()
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = SamplerActor::spawn(SamplerConfig {
            base_url: server.url(), model: "test-model".into(), api_backend: ApiBackend::ChatCompletions,
            max_retries: Some(0), ..Default::default()
        }, RetryPolicy::default(), tx);
        let mut fixture = make_replay_send_update_fixture().await;
        let actor = Arc::new(fixture.actor);
        *actor.current_prompt_id.lock().unwrap() = Some("auth-prompt".into());
        let request = RequestId::from("auth-request");
        let deadline = RetryDeadline::after(std::time::Duration::from_secs(60)).unwrap();
        actor.schedule_sampling_retry(&request, &deadline, crate::extensions::notification::RetryState::Retrying {
            attempt: 1, max_retries: 50, reason: "credential recovery".into(), error_type: None,
            next_attempt_at_ms: deadline.projection().wall_deadline_ms,
        });
        let scheduled = take_notification(&mut fixture.event_rx);
        let owner = lifecycle(&scheduled).owner;
        pace_uncharged_resubmit(RecoveredStore::SessionToken, Some(&manager), &deadline).await.unwrap();
        if ActiveAuthBackend::default().is_xai_authority() {
            assert!(deadline.remaining() > std::time::Duration::from_secs(59));
        } else {
            assert_eq!(deadline.remaining(), std::time::Duration::ZERO);
        }
        handle.submit(request.clone(), ConversationRequest {
            items: vec![ConversationItem::user("hi")], ..Default::default()
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let event = rx.recv().await.expect("sampler event");
                if matches!(&event, SamplingEvent::LocalRequest {
                    evidence: xai_grok_sampler::request_capture::LocalRequestEvidence::DispatchStarted { .. }, ..
                }) {
                    actor.handle_sampling_event(event).await;
                    break;
                }
                assert!(!matches!(&event, SamplingEvent::FirstToken { .. } | SamplingEvent::ChannelToken { .. }));
                actor.handle_sampling_event(event).await;
            }
        }).await.unwrap();
        let dispatched = take_notification(&mut fixture.event_rx);
        assert_eq!(lifecycle(&dispatched).owner, owner);
        assert!(matches!(lifecycle(&dispatched).phase, SamplingPhase::DispatchStarted { .. }));
        handle.cancel(request);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_cancel_in_backoff_settles_only_the_reserved_attempt_and_revokes_its_drain() {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = make_replay_send_update_fixture().await;
        let actor = Arc::new(fixture.actor);
        *actor.current_prompt_id.lock().unwrap() = Some("cancel-prompt".into());
        let request = RequestId::from("cancel-request");
        actor.register_sampling_request(&request);
        let (tx, rx) = tokio::sync::oneshot::channel();
        actor.turn_stream_drained.lock().insert(request.clone(), StreamOwnership::with_waiter(Some(tx)));
        actor.handle_sampling_event(retry(&request, 2)).await;
        let scheduled = take_notification(&mut fixture.event_rx);
        actor.handle_sampling_event(SamplingEvent::LocalRequest {
            request_id: request.clone(), attempt: RequestAttemptId(2),
            evidence: xai_grok_sampler::request_capture::LocalRequestEvidence::Finished {
                outcome: xai_grok_sampler::request_capture::LocalAttemptOutcome::Cancelled,
            },
        }).await;
        let cancelled = take_notification(&mut fixture.event_rx);
        assert_eq!(lifecycle(&cancelled).owner, lifecycle(&scheduled).owner);
        assert_eq!(lifecycle(&cancelled).phase, SamplingPhase::Finished { outcome: SamplingOutcome::Cancelled });
        actor.handle_sampling_event(SamplingEvent::Failed {
            request_id: request.clone(), error: (&SamplingError::EventStreamError("cancelled".into())).into(),
        }).await;
        assert!(rx.await.is_err(), "cancelled requests release the waiter as revoked");
        assert!(!actor.turn_stream_drained.lock().contains_key(&request));
        actor.handle_sampling_event(SamplingEvent::LocalRequest {
            request_id: request.clone(), attempt: RequestAttemptId(2),
            evidence: xai_grok_sampler::request_capture::LocalRequestEvidence::DispatchStarted { started_at: Default::default() },
        }).await;
        assert!(fixture.event_rx.try_recv().is_err());
        assert!(!actor.accepts_sampling_owner(&lifecycle(&scheduled).owner));
    }).await;
}
