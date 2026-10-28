use super::*;
use crate::request_capture::LocalRequestEvidence;
use crate::retry_deadline::RetryScheduleCause;
use xai_grok_test_support::{MockInferenceServer, MockModelEntry, ScriptedResponse};

fn failure(status: u16, delay: &str) -> ScriptedResponse {
    let mut response = ScriptedResponse::text(status, "original provider failure");
    response.headers.push(("retry-after".into(), delay.into()));
    response
}

fn config(server: &MockInferenceServer, backend: ApiBackend) -> SamplerConfig {
    SamplerConfig {
        base_url: server.url(), model: "test-model".into(), api_backend: backend,
        max_retries: Some(2), ..Default::default()
    }
}

fn request() -> ConversationRequest {
    ConversationRequest::from_items(vec![xai_grok_sampling_types::ConversationItem::user("retry input")])
}

#[tokio::test]
async fn impossible_http_delay_is_terminal_before_reservation_for_each_backend() {
    for (backend, path) in [
        (ApiBackend::ChatCompletions, "/v1/chat/completions"),
        (ApiBackend::Responses, "/v1/responses"),
        (ApiBackend::Messages, "/v1/messages"),
    ] {
        for (status, seconds) in [(429, u64::MAX), (503, u64::MAX), (503, u64::MAX / 1000 + 1)] {
            let server = MockInferenceServer::start_with_models(vec![MockModelEntry::new("test-model")]).await.unwrap();
            server.enqueue_response(path, failure(status, &seconds.to_string()));
            let (tx, mut rx) = mpsc::unbounded_channel();
            let (completion_tx, completion_rx) = oneshot::channel();
            let id = RequestId::random();
            run_request_task(id.clone(), request(), config(&server, backend.clone()), RetryPolicy::default(),
                tx, CancellationToken::new(), Some(completion_tx)).await;
            let collected = completion_rx.await.unwrap();
            let error = collected.result.unwrap_err();
            let SamplingError::RetrySchedule { original, requested_backoff, cause } = &error else {
                panic!("expected scheduling failure: {error:?}");
            };
            assert_eq!(*requested_backoff, Duration::from_secs(seconds));
            if seconds == u64::MAX {
                assert!(matches!(cause, RetryScheduleCause::InstantRange));
            } else {
                assert!(matches!(cause, RetryScheduleCause::Deadline(tokio::time::error::DeadlineError::TickOutOfRange)));
            }
            assert!(matches!(original.as_ref(), SamplingError::Api { status: original_status, retry_after_secs: Some(original_seconds), .. } if original_status.as_u16() == status && *original_seconds == seconds));
            assert!(!error.is_retryable());
            let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            assert!(!events.iter().any(|event| matches!(event, SamplingEvent::Retrying { .. } | SamplingEvent::Completed { .. })));
            assert_eq!(events.iter().filter(|event| matches!(event, SamplingEvent::LocalRequest {
                attempt: RequestAttemptId(1), evidence: LocalRequestEvidence::DispatchStarted { .. }, ..
            })).count(), 1);
            assert!(!events.iter().any(|event| matches!(event, SamplingEvent::LocalRequest { attempt, .. } if attempt.0 > 1)));
            let info = events.iter().find_map(|event| match event {
                SamplingEvent::Failed { error, .. } => Some(error), _ => None,
            }).unwrap();
            let wire = serde_json::to_value(info).unwrap();
            let decoded: SamplingErrorInfo = serde_json::from_value(wire).unwrap();
            let details = decoded.retry_schedule.as_ref().unwrap();
            assert_eq!(details.requested_backoff, Duration::from_secs(seconds));
            assert_eq!(details.original.status_code, Some(status));
            assert_eq!(serde_json::to_value(details.cause).unwrap(), serde_json::to_value(cause).unwrap());
            assert_eq!(server.request_count(), 1);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn malformed_header_retains_integer_only_contract_and_waits_normally() {
    let server = MockInferenceServer::start_with_models(vec![MockModelEntry::new("test-model")]).await.unwrap();
    server.enqueue_response("/v1/responses", failure(503, "not integer seconds"));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let id = RequestId::random();
    let task = tokio::spawn(run_request_task(id, request(), config(&server, ApiBackend::Responses),
        RetryPolicy::default(), tx, cancel.clone(), None));
    let deadline = loop {
        match rx.recv().await.unwrap() {
            SamplingEvent::Retrying { deadline, .. } => break deadline,
            SamplingEvent::Failed { error, .. } => panic!("unexpected terminal error: {error:?}"),
            _ => {}
        }
    };
    assert!(deadline.remaining() > Duration::ZERO);
    assert_eq!(server.request_count(), 1);
    cancel.cancel();
    task.await.unwrap();
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(!events.iter().any(|event| matches!(event, SamplingEvent::Completed { .. })));
    assert_eq!(server.request_count(), 1);
}

#[test]
fn driver_shutdown_retains_native_cause_and_never_dispatches_reserved_retry() {
    let transport = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let server = transport.block_on(MockInferenceServer::start_with_models(vec![MockModelEntry::new("test-model")])).unwrap();
    server.enqueue_response("/v1/responses", failure(503, "1200"));
    let owner = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (completion_tx, completion_rx) = oneshot::channel();
    let mut task = Box::pin(run_request_task(RequestId::random(), request(), config(&server, ApiBackend::Responses),
        RetryPolicy::default(), tx, CancellationToken::new(), Some(completion_tx)));
    let bound_deadline = owner.block_on(async {
        loop {
            tokio::select! {
                _ = &mut task => panic!("request finished before its scheduled wait"),
                event = rx.recv() => if let SamplingEvent::Retrying { deadline, next_attempt, .. } = event.unwrap() {
                    assert_eq!(next_attempt, RequestAttemptId(2));
                    break deadline;
                }
            }
        }
    });
    drop(owner);
    transport.block_on(task);
    let error = transport.block_on(completion_rx).unwrap().result.unwrap_err();
    let SamplingError::RetrySchedule { requested_backoff, original, cause } = error else { panic!("native scheduling fault lost"); };
    assert_eq!(requested_backoff, Duration::from_secs(1200));
    assert!(matches!(original.as_ref(), SamplingError::Api { status, .. } if status.as_u16() == 503));
    assert!(matches!(cause, RetryScheduleCause::Wait(error) if error.is_shutdown()));
    assert!(transport.block_on(bound_deadline.clone().wait()).unwrap_err().is_shutdown());
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(!events.iter().any(|event| matches!(event, SamplingEvent::Completed { .. } | SamplingEvent::LocalRequest {
        attempt: RequestAttemptId(2), evidence: LocalRequestEvidence::DispatchStarted { .. }, ..
    })));
    let terminal = events.iter().find_map(|event| match event {
        SamplingEvent::Failed { error, .. } => Some(error), _ => None,
    }).unwrap();
    let decoded: SamplingErrorInfo = serde_json::from_value(serde_json::to_value(terminal).unwrap()).unwrap();
    assert!(matches!(decoded.to_error(), SamplingError::RetrySchedule { cause: RetryScheduleCause::Wait(error), .. } if error.is_shutdown()));
    assert_eq!(server.request_count(), 1);

    server.enqueue_response("/v1/responses", failure(503, "1200"));
    let (tx, mut fresh_events) = mpsc::unbounded_channel();
    let mut fresh_config = config(&server, ApiBackend::Responses);
    fresh_config.max_retries = Some(0);
    transport.block_on(run_request_task(RequestId::random(), request(), fresh_config, RetryPolicy::default(),
        tx, CancellationToken::new(), None));
    let fresh: Vec<_> = std::iter::from_fn(|| fresh_events.try_recv().ok()).collect();
    assert!(fresh.iter().any(|event| matches!(event, SamplingEvent::LocalRequest {
        attempt: RequestAttemptId(1), evidence: LocalRequestEvidence::DispatchStarted { .. }, ..
    })));
    assert!(!fresh.iter().any(|event| matches!(event, SamplingEvent::Retrying { .. })));
    assert_eq!(server.request_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn integer_twelve_hundred_second_http_delays_retain_the_real_wait_anchor() {
    for status in [429, 503] {
        let server = MockInferenceServer::start_with_models(vec![MockModelEntry::new("test-model")]).await.unwrap();
        server.enqueue_response("/v1/responses", failure(status, "1200"));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_request_task(RequestId::random(), request(), config(&server, ApiBackend::Responses),
            RetryPolicy::default(), tx, cancel.clone(), None));
        let deadline = loop {
            match rx.recv().await.unwrap() {
                SamplingEvent::Retrying { backoff_ms, deadline, .. } => {
                    assert_eq!(backoff_ms, 1_200_000);
                    break deadline;
                }
                SamplingEvent::Failed { error, .. } => panic!("unexpected terminal error: {error:?}"),
                _ => {}
            }
        };
        let original_anchor = tokio::time::Instant::now() + Duration::from_secs(1200);
        tokio::time::advance(Duration::from_secs(90)).await;
        assert_eq!(deadline.remaining(), Duration::from_secs(1110));
        assert_eq!(server.request_count(), 1);
        deadline.clone().wait().await.unwrap();
        assert_eq!(tokio::time::Instant::now(), original_anchor);
        cancel.cancel();
        task.await.unwrap();
    }
}
