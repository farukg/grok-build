use super::*;
use axum::response::IntoResponse;
use xai_grok_sampler::request_capture::{LocalRequestEvidence, RequestAttemptId};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transport_then_doom_retries_use_distinct_cumulative_dispatch_owners() {
    let count = Arc::new(AtomicU32::new(0));
    let calls = count.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move || {
            let calls = calls.clone();
            async move {
                let events = match calls.fetch_add(1, Ordering::SeqCst) {
                    0 => {
                        return (StatusCode::SERVICE_UNAVAILABLE, "temporary failure")
                            .into_response();
                    }
                    1 => sse::responses_api_doom_loop_terminal_only_events(
                        &["tail_repetition:8@thinking"],
                        "repeated thought",
                        "discarded response",
                        "test-model",
                    ),
                    _ => sse::responses_api_reasoning_and_text_events(
                        "fresh thought",
                        "accepted response",
                        "test-model",
                    ),
                };
                Sse::new(stream::iter(
                    sse_events_to_axum(events)
                        .into_iter()
                        .map(Ok::<_, std::convert::Infallible>),
                ))
                .into_response()
            }
        }),
    );
    let server = MockServer::spawn(app).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let config = responses_config(server.base_url(), Some(DoomLoopRecoveryPolicy::default()));
    let handle = SamplerActor::spawn(config, RetryPolicy::default(), tx);
    let request = RequestId::random();
    let (response, metrics) = handle
        .submit_and_collect(request.clone(), user_request("hi"))
        .await
        .unwrap();
    let events = drain_until_terminal(&mut rx, Duration::from_secs(1)).await;
    let dispatches: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SamplingEvent::LocalRequest {
                request_id,
                attempt,
                evidence: LocalRequestEvidence::DispatchStarted { .. },
            } => {
                assert_eq!(request_id, &request);
                Some(*attempt)
            }
            _ => None,
        })
        .collect();
    let retries: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SamplingEvent::Retrying {
                request_id,
                attempt,
                next_attempt,
                ..
            } => {
                assert_eq!(request_id, &request);
                Some((*attempt, *next_attempt))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        dispatches,
        [
            RequestAttemptId(1),
            RequestAttemptId(2),
            RequestAttemptId(3)
        ]
    );
    assert_eq!(
        retries,
        [(1, RequestAttemptId(2)), (1, RequestAttemptId(3))]
    );
    assert_eq!(response.assistant_text(), "accepted response");
    assert_eq!(metrics.attempts, 3);
    assert_eq!(count.load(Ordering::SeqCst), 3);
    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_dispatch_is_observable_while_the_server_has_not_produced_a_token() {
    let count = Arc::new(AtomicU32::new(0));
    let release = Arc::new(tokio::sync::Notify::new());
    let calls = count.clone();
    let gate = release.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let calls = calls.clone();
            let gate = gate.clone();
            async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return (StatusCode::SERVICE_UNAVAILABLE, "temporary failure").into_response();
                }
                gate.notified().await;
                Sse::new(stream::iter(
                    sse::chat_completion_events("accepted", "test-model")
                        .into_iter()
                        .map(Ok::<_, std::convert::Infallible>),
                ))
                .into_response()
            }
        }),
    );
    let server = MockServer::spawn(app).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = SamplerActor::spawn(
        test_config(server.base_url(), "test-model"),
        RetryPolicy::default(),
        tx,
    );
    let request = RequestId::random();
    let task_handle = handle.clone();
    let task_request = request.clone();
    let result = tokio::spawn(async move {
        task_handle
            .submit_and_collect(task_request, user_request("hi"))
            .await
    });
    let mut retry_owner = None;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rx.recv().await.unwrap() {
                SamplingEvent::Retrying { next_attempt, .. } => retry_owner = Some(next_attempt),
                SamplingEvent::LocalRequest {
                    request_id,
                    attempt,
                    evidence: LocalRequestEvidence::DispatchStarted { .. },
                } if attempt == RequestAttemptId(2) => {
                    assert_eq!(request_id, request);
                    assert_eq!(retry_owner, Some(attempt));
                    break;
                }
                SamplingEvent::ChannelToken { .. } | SamplingEvent::FirstToken { .. } => {
                    panic!("server has not been released")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    release.notify_one();
    assert_eq!(
        result.await.unwrap().unwrap().0.assistant_text(),
        "accepted"
    );
    server.shutdown();
}

#[tokio::test(start_paused = true)]
async fn actual_429_scheduler_keeps_the_entire_twenty_minute_retry_after() {
    let count = Arc::new(AtomicU32::new(0));
    let calls = count.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let calls = calls.clone();
            async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", "1200")],
                        "capacity",
                    )
                        .into_response();
                }
                Sse::new(stream::iter(
                    sse::chat_completion_events("accepted", "test-model")
                        .into_iter()
                        .map(Ok::<_, std::convert::Infallible>),
                ))
                .into_response()
            }
        }),
    );
    let server = MockServer::spawn(app).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = SamplerActor::spawn(
        test_config(server.base_url(), "test-model"),
        RetryPolicy::default(),
        tx,
    );
    let request = RequestId::random();
    let task =
        tokio::spawn(async move { handle.submit_and_collect(request, user_request("hi")).await });
    let deadline = loop {
        if let SamplingEvent::Retrying {
            backoff_ms,
            deadline,
            next_attempt,
            ..
        } = rx.recv().await.unwrap()
        {
            assert_eq!(backoff_ms, 1_200_000);
            assert_eq!(next_attempt, RequestAttemptId(2));
            break deadline;
        }
    };
    assert_eq!(deadline.remaining(), Duration::from_secs(1200));
    tokio::time::advance(Duration::from_secs(1199)).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(deadline.remaining(), Duration::from_secs(1));
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(task.await.unwrap().unwrap().0.assistant_text(), "accepted");
    assert_eq!(count.load(Ordering::SeqCst), 2);
    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_long_retry_wait_terminals_its_reserved_attempt_without_dispatch() {
    let count = Arc::new(AtomicU32::new(0));
    let calls = count.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", "1200")],
                    "capacity",
                )
            }
        }),
    );
    let server = MockServer::spawn(app).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = SamplerActor::spawn(
        test_config(server.base_url(), "test-model"),
        RetryPolicy::default(),
        tx,
    );
    let request = RequestId::random();
    handle.submit(request.clone(), user_request("hi"));
    let reserved = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let SamplingEvent::Retrying { next_attempt, .. } = rx.recv().await.unwrap() {
                break next_attempt;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(reserved, RequestAttemptId(2));
    handle.cancel(request.clone());
    let events = drain_until_terminal(&mut rx, Duration::from_secs(5)).await;
    assert!(events.iter().any(|event| matches!(event,
        SamplingEvent::LocalRequest {
            request_id, attempt,
            evidence: LocalRequestEvidence::Finished { outcome: xai_grok_sampler::request_capture::LocalAttemptOutcome::Cancelled },
        } if request_id == &request && *attempt == reserved
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        SamplingEvent::LocalRequest {
            evidence: LocalRequestEvidence::DispatchStarted { .. },
            ..
        }
    )));
    assert!(matches!(events.last(), Some(SamplingEvent::Failed { .. })));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    server.shutdown();
}
