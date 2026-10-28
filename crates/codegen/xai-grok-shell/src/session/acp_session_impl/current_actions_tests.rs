use super::*;
use crate::session::current_actions::{
    CurrentActions, CurrentActionsSnapshot, CurrentSamplingPhase, SnapshotBinding, SourceVersion,
    VERSION_META_KEY,
};

fn binding(actor: &SessionActor) -> SnapshotBinding {
    SnapshotBinding {
        session_id: actor.session_info.id.0.to_string(),
        binding_epoch: 7,
        load_generation: 3,
    }
}

fn current(snapshot: CurrentActionsSnapshot) -> CurrentActions {
    let CurrentActionsSnapshot::Current(mut current) = snapshot else {
        panic!("resident source must be available");
    };
    // The native cursor is process-wide; unrelated sessions may allocate between reads.
    current.event_frontier = 0;
    current
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn real_retry_producer_is_observable_without_a_viewer_and_keeps_its_scheduler_point() {
    use xai_grok_sampler::retry_deadline::RetryClockMapping;
    use xai_grok_sampler::{ApiBackend, RetryPolicy, SamplerActor, SamplerConfig, SamplingEvent};
    use xai_grok_test_support::{MockInferenceServer, MockModelEntry, ScriptedResponse};
    tokio::task::LocalSet::new().run_until(async {
        let server = MockInferenceServer::start_with_models(vec![MockModelEntry::new("test-model")]).await.unwrap();
        let mut reply = ScriptedResponse::text(503, "temporary failure");
        reply.headers.push(("retry-after".into(), "1200".into()));
        server.enqueue_response("/v1/responses", reply);
        let (events, mut sampling) = tokio::sync::mpsc::unbounded_channel();
        let sampler = SamplerActor::spawn(SamplerConfig {
            base_url: server.url(), model: "test-model".into(), api_backend: ApiBackend::Responses,
            max_retries: Some(2), ..Default::default()
        }, RetryPolicy::default(), events);
        let mut fixture = make_replay_send_update_fixture().await;
        let actor = Arc::new(fixture.actor);
        actor.notifications.gateway_enabled.store(false, std::sync::atomic::Ordering::Relaxed);
        *actor.current_prompt_id.lock().unwrap() = Some("actual-producer".into());
        let request = xai_grok_sampler::RequestId::random();
        actor.register_sampling_request(&request);
        actor.turn_stream_drained.lock().insert(request.clone(), StreamOwnership::with_waiter(None));
        sampler.submit(request.clone(), ConversationRequest { items: vec![ConversationItem::user("fixture input")], ..Default::default() });
        let deadline = loop {
            let event = sampling.recv().await.unwrap();
            let retry = match &event { SamplingEvent::Retrying { deadline, backoff_ms, .. } => { assert_eq!(*backoff_ms, 1_200_000); Some(deadline.clone()) }, _ => None };
            actor.handle_sampling_event(event).await;
            if let Some(deadline) = retry { break deadline; }
        };
        let reader = actor.current_actions_reader();
        let tag = binding(&actor);
        let captured = reader.snapshot(tag.clone());
        let observed = current(captured.clone());
        let [request] = observed.sampling.as_slice() else { panic!("one real reserved retry"); };
        let CurrentSamplingPhase::RetryScheduled { deadline: projection } = &request.phase else { panic!("retry is not a dispatch"); };
        assert_eq!(projection, &deadline.projection());
        if let RetryClockMapping::Correlated { earliest_due_at, latest_due_at } = projection.map_local() {
            let original = tokio::time::Instant::now() + deadline.remaining();
            assert!(earliest_due_at <= original && original <= latest_due_at);
        }
        let before = actor.current_actions.version();
        let native_turn = actor.current_turn_number.get();
        let _actor_lock = actor.state.lock().await;
        assert_eq!(current(reader.snapshot(tag.clone())), current(captured.clone()));
        assert_eq!(actor.current_actions.version(), before);
        assert_eq!(actor.current_turn_number.get(), native_turn);
        drop(_actor_lock);
        tokio::time::advance(Duration::from_millis(50)).await;
        let cold_reader = actor.current_actions_reader();
        assert_eq!(current(cold_reader.snapshot(tag.clone())), current(captured.clone()), "attach must not rebase a retained scheduler anchor");
        sampler.cancel(request.owner.request_id.clone());
        loop {
            let event = sampling.recv().await.unwrap();
            let terminal = matches!(event, SamplingEvent::Failed { .. });
            actor.handle_sampling_event(event).await;
            if terminal { break; }
        }
        let mut terminal_version = None;
        let mut terminal_sequence = None;
        while let Ok(SessionEvent::Notification(notification)) = fixture.event_rx.try_recv() {
            if let SessionNotification::Xai(notification) = notification
                && matches!(notification.update, XaiSessionUpdate::SamplingLifecycle { lifecycle: crate::extensions::sampling_lifecycle::SamplingLifecycle { phase: crate::extensions::sampling_lifecycle::SamplingPhase::Finished { .. }, .. } })
            {
                let meta = notification.meta.unwrap();
                terminal_sequence = Some(meta["eventId"].as_str().unwrap().rsplit('-').next().unwrap().parse::<u64>().unwrap());
                terminal_version = Some(serde_json::from_value::<SourceVersion>(meta[VERSION_META_KEY].clone()).unwrap());
            }
        }
        let terminal_version = terminal_version.expect("actual terminal must carry the source frontier");
        assert!(!captured.is_current_for(&tag, Some(terminal_version), None));
        assert!(!captured.is_current_for(&tag, None, terminal_sequence), "the existing native cursor independently fences a later real live event");
        assert!(current(reader.snapshot(tag)).sampling.is_empty(), "terminal-before-load is empty, not an adopted retry");
    }).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn real_tool_snapshot_preserves_registration_and_execution_across_absent_viewer_and_late_replay()
 {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut fixture = make_replay_send_update_fixture().await;
            let release = Arc::new(tokio::sync::Notify::new());
            install_fixture_tool(
                &fixture.actor,
                ToolKind::ImageGen,
                TimingFixtureSchedule::Held(release.clone()),
            );
            fixture
                .actor
                .notifications
                .gateway_enabled
                .store(false, std::sync::atomic::Ordering::Relaxed);
            let actor = Arc::new(fixture.actor);
            let reader = actor.current_actions_reader();
            let tag = binding(&actor);
            let execute = actor.execute_tool_calls(vec![call("real-invocation")], None, None);
            let observe = async {
                let mut base = None;
                let started = loop {
                    let SessionEvent::Notification(notification) =
                        fixture.event_rx.recv().await.unwrap()
                    else {
                        continue;
                    };
                    if let SessionNotification::Acp(acp) = &notification {
                        match &acp.update {
                            acp::SessionUpdate::ToolCall(_) => base = Some(notification.clone()),
                            acp::SessionUpdate::ToolCallUpdate(update) => {
                                if let started @ ToolExecutionTiming::Started { .. } =
                                    timing_from_meta(update.meta.as_ref()).unwrap()
                                {
                                    break started;
                                }
                            }
                            _ => {}
                        }
                    }
                };
                let source = current(reader.snapshot(tag.clone()));
                let [tool] = source.tools.as_slice() else {
                    panic!("one genuine invocation");
                };
                assert_eq!(tool.executions, [started]);
                let registration = tool.registered_at;
                tokio::time::advance(Duration::from_secs(1)).await;
                let later = current(actor.current_actions_reader().snapshot(tag.clone()));
                assert_eq!(later.tools[0].registered_at, registration);
                assert_eq!(later.tools[0].executions, [started]);
                assert_eq!(
                    later.version, source.version,
                    "query and elapsed time are not work transitions"
                );
                let encoded = serde_json::to_string(&later).unwrap();
                assert!(
                    !encoded.contains("/tmp/timing-fixture-shared"),
                    "actual tool arguments are not continuation metadata"
                );
                for private_field in [
                    "rawInput",
                    "rawOutput",
                    "arguments",
                    "prompt_blocks",
                    "questions",
                ] {
                    assert!(!encoded.contains(private_field));
                }
                release.notify_one();
                tokio::time::advance(Duration::from_secs(1)).await;
                let (_, finished) = next_timing(&mut fixture.event_rx).await;
                let ToolExecutionTiming::Started {
                    execution_id,
                    started_at,
                } = started
                else {
                    panic!("actual start");
                };
                assert_eq!(
                    finished,
                    ToolExecutionTiming::Finished {
                        execution_id,
                        started_at,
                        duration_ms: 2_000,
                        termination: ToolExecutionTermination::Returned,
                    }
                );
                base.unwrap()
            };
            let (result, base) = tokio::join!(execute, observe);
            assert!(result.is_ok());
            assert!(current(reader.snapshot(tag.clone())).tools.is_empty());
            let before = reader.snapshot(tag.clone());
            let mut replay = base.into_acp();
            replay
                .meta
                .get_or_insert_with(acp::Meta::new)
                .insert("isReplay".into(), true.into());
            actor.emit_notification_direct(replay).await;
            assert_eq!(
                current(reader.snapshot(tag.clone())),
                current(before),
                "historical registration is not source admission"
            );
            let observed = reader.snapshot(tag);
            let encoded = serde_json::to_value(&observed).unwrap();
            let decoded: CurrentActionsSnapshot = serde_json::from_value(encoded.clone()).unwrap();
            assert_eq!(decoded, observed);
            let mut invalid = encoded;
            invalid["version"]["incarnation"] = serde_json::json!(uuid::Uuid::nil());
            assert!(serde_json::from_value::<CurrentActionsSnapshot>(invalid).is_err());
        })
        .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn permission_snapshot_uses_the_real_wait_lifetime_and_never_answers_the_request() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let mut fixture = make_replay_send_update_fixture().await;
            install_fixture_tool(
                &fixture.actor,
                ToolKind::Other,
                TimingFixtureSchedule::Immediate,
            );
            let mut permissions = permission_gate(&mut fixture.actor);
            let actor = Arc::new(fixture.actor);
            let reader = actor.current_actions_reader();
            let tag = binding(&actor);
            let execute = actor.execute_tool_calls(vec![call("waiting")], None, None);
            let observe = async {
                let PermissionCommand::Request { respond_to, .. } =
                    permissions.recv().await.unwrap()
                else {
                    panic!("real reverse request");
                };
                let before = current(reader.snapshot(tag.clone()));
                assert_eq!(before.interactions.len(), 1);
                assert_eq!(
                    before.interactions[0].kind,
                    crate::session::pending_interaction::PendingKind::Permission
                );
                assert!(
                    before.tools[0].executions.is_empty(),
                    "permission wait must not claim execution"
                );
                let stamp = before.interactions[0].source.clone();
                tokio::time::advance(Duration::from_secs(30)).await;
                let later = current(reader.snapshot(tag.clone()));
                assert_eq!(later.interactions[0].source, stamp);
                assert_eq!(later.version, before.version);
                assert!(
                    !respond_to.is_closed(),
                    "observation cannot transfer or answer the permission recipient"
                );
                respond_to
                    .send(PermissionResolution {
                        decision: Decision::Allow,
                        event: None,
                    })
                    .unwrap();
                let (_, start) = next_timing(&mut fixture.event_rx).await;
                assert!(matches!(start, ToolExecutionTiming::Started { .. }));
                assert!(
                    current(reader.snapshot(tag.clone()))
                        .interactions
                        .is_empty()
                );
                tokio::time::advance(Duration::from_secs(2)).await;
            };
            let (result, ()) = tokio::join!(execute, observe);
            assert!(result.is_ok());
            assert!(current(reader.snapshot(tag)).tools.is_empty());
        })
        .await;
}
