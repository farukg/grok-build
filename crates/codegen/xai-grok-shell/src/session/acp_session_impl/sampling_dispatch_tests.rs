use super::*;
use crate::extensions::sampling_lifecycle::{
    SAMPLING_LIFECYCLE_META_KEY, SamplingLifecycle, SamplingPhase,
};
use crate::session::acp_session::replay_buffer_send_update_tests::make_replay_send_update_fixture;
use crate::session::current_actions::{
    CurrentActionsSnapshot, CurrentSamplingPhase, SnapshotBinding,
};
use xai_grok_sampler::request_capture::{LocalRequestEvidence, RequestAttemptId};
use xai_grok_sampler::{ApiBackend, RetryPolicy, SamplerActor, SamplerConfig, SamplingEvent};
use xai_grok_test_support::{MockInferenceServer, MockModelEntry, ScriptedResponse};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn transport_dispatch_point_survives_twenty_seconds_delivery_and_cold_snapshot() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let server =
                MockInferenceServer::start_with_models(vec![MockModelEntry::new("test-model")])
                    .await
                    .unwrap();
            let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
            let sampler = SamplerActor::spawn(
                SamplerConfig {
                    base_url: server.url(),
                    model: "test-model".into(),
                    api_backend: ApiBackend::Responses,
                    max_retries: Some(0),
                    ..Default::default()
                },
                RetryPolicy::default(),
                tx,
            );
            let mut fixture = make_replay_send_update_fixture().await;
            let actor = Arc::new(fixture.actor);
            actor
                .notifications
                .gateway_enabled
                .store(false, std::sync::atomic::Ordering::Relaxed);
            *actor.current_prompt_id.lock().unwrap() = Some("dispatch-origin".into());
            let id = xai_grok_sampler::RequestId::random();
            actor.register_sampling_request(&id);
            actor
                .turn_stream_drained
                .lock()
                .insert(id.clone(), StreamOwnership::with_waiter(None));
            let before_submit = tokio::time::Instant::now();
            sampler.submit(
                id.clone(),
                ConversationRequest::from_items(vec![ConversationItem::user("input")]),
            );
            let (dispatch, original) = loop {
                let event = events.recv().await.unwrap();
                match &event {
                    SamplingEvent::LocalRequest {
                        evidence: LocalRequestEvidence::DispatchStarted { started_at },
                        ..
                    } => {
                        let point = *started_at;
                        break (event, point);
                    }
                    SamplingEvent::FirstToken { .. } | SamplingEvent::ChannelToken { .. } => {
                        panic!("dispatch must precede tokens")
                    }
                    _ => actor.handle_sampling_event(event).await,
                }
            };
            let before_delivery_delay = tokio::time::Instant::now();
            tokio::time::advance(Duration::from_secs(20)).await;
            if let xai_monotonic_clock::LocalClockMapping::Correlated {
                earliest_at,
                latest_at,
            } = original.map_local()
            {
                assert!(earliest_at <= before_delivery_delay && before_submit <= latest_at);
                assert!(latest_at < tokio::time::Instant::now() - Duration::from_secs(19));
            }
            actor.handle_sampling_event(dispatch).await;
            let binding = SnapshotBinding {
                session_id: actor.session_info.id.0.to_string(),
                binding_epoch: 5,
                load_generation: 9,
            };
            let CurrentActionsSnapshot::Current(snapshot) =
                actor.current_actions_reader().snapshot(binding.clone())
            else {
                panic!("resident source");
            };
            let [submitted] = snapshot.sampling.as_slice() else {
                panic!("one dispatched owner");
            };
            assert_eq!(submitted.owner.request_id, id);
            assert_eq!(submitted.owner.attempt, RequestAttemptId(1));
            assert_eq!(
                submitted.phase,
                CurrentSamplingPhase::Submitted {
                    started_at: original
                }
            );
            let mut live_point = None;
            while let Ok(SessionEvent::Notification(notification)) = fixture.event_rx.try_recv() {
                if let SessionNotification::Xai(notification) = notification
                    && let Some(meta) = notification.meta
                    && let Some(value) = meta.get(SAMPLING_LIFECYCLE_META_KEY)
                {
                    let lifecycle: SamplingLifecycle =
                        serde_json::from_value(value.clone()).unwrap();
                    if let SamplingPhase::DispatchStarted { started_at } = lifecycle.phase {
                        assert_eq!(lifecycle.owner, submitted.owner);
                        live_point = Some(started_at);
                    }
                }
            }
            assert_eq!(live_point, Some(original));
            tokio::time::advance(Duration::from_secs(20)).await;
            let CurrentActionsSnapshot::Current(cold) =
                actor.current_actions_reader().snapshot(binding)
            else {
                panic!("resident source");
            };
            assert_eq!(cold.sampling, snapshot.sampling);
            assert_eq!(cold.version, snapshot.version);
            sampler.cancel(id);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn impossible_http_schedule_retires_the_actual_source_owner_without_a_retry() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let server =
                MockInferenceServer::start_with_models(vec![MockModelEntry::new("test-model")])
                    .await
                    .unwrap();
            let mut reply = ScriptedResponse::text(503, "original provider failure");
            reply
                .headers
                .push(("retry-after".into(), u64::MAX.to_string()));
            server.enqueue_response("/v1/responses", reply);
            let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
            let sampler = SamplerActor::spawn(
                SamplerConfig {
                    base_url: server.url(),
                    model: "test-model".into(),
                    api_backend: ApiBackend::Responses,
                    max_retries: Some(2),
                    ..Default::default()
                },
                RetryPolicy::default(),
                tx,
            );
            let fixture = make_replay_send_update_fixture().await;
            let actor = Arc::new(fixture.actor);
            *actor.current_prompt_id.lock().unwrap() = Some("failed-scheduler".into());
            let id = xai_grok_sampler::RequestId::random();
            actor.register_sampling_request(&id);
            actor
                .turn_stream_drained
                .lock()
                .insert(id.clone(), StreamOwnership::with_waiter(None));
            sampler.submit(
                id.clone(),
                ConversationRequest::from_items(vec![ConversationItem::user("input")]),
            );
            loop {
                let event = events.recv().await.unwrap();
                assert!(!matches!(
                    &event,
                    SamplingEvent::Retrying { .. } | SamplingEvent::Completed { .. }
                ));
                let terminal = matches!(&event, SamplingEvent::Failed { .. });
                if let SamplingEvent::Failed { error, .. } = &event {
                    assert_eq!(
                        error.kind,
                        xai_grok_sampler::SamplingErrorKind::RetrySchedule
                    );
                    assert_eq!(
                        error.retry_schedule.as_ref().unwrap().original.status_code,
                        Some(503)
                    );
                }
                actor.handle_sampling_event(event).await;
                if terminal {
                    break;
                }
            }
            assert!(actor.sampling_request_discarded(&id));
            assert_eq!(
                actor.sampling_owner(&id).unwrap().attempt,
                RequestAttemptId(1)
            );
            let CurrentActionsSnapshot::Current(snapshot) =
                actor.current_actions_reader().snapshot(SnapshotBinding {
                    session_id: actor.session_info.id.0.to_string(),
                    binding_epoch: 1,
                    load_generation: 1,
                })
            else {
                panic!("resident source");
            };
            assert!(snapshot.sampling.is_empty());
            assert_eq!(server.request_count(), 1);
        })
        .await;
}

#[test]
fn legacy_dispatch_wire_has_unknown_provenance_without_inventing_a_point() {
    let phase: SamplingPhase =
        serde_json::from_value(serde_json::json!({ "phase": "dispatch_started" })).unwrap();
    assert_eq!(
        phase,
        SamplingPhase::DispatchStarted {
            started_at: Default::default()
        }
    );
    let submitted: CurrentSamplingPhase =
        serde_json::from_value(serde_json::json!({ "phase": "submitted" })).unwrap();
    assert_eq!(
        submitted,
        CurrentSamplingPhase::Submitted {
            started_at: Default::default()
        }
    );
}
