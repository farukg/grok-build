use super::*;
use crate::SamplingEvent;
use crate::request_capture::{CaptureAttempt, LocalAttemptOutcome, LocalRequestEvidence, PayloadRepresentation, RequestAttemptId, RequestBodyCache};
use axum::{Router, body::Bytes, routing::post};
use tokio::sync::mpsc;
use xai_grok_sampling_types::{ConversationItem, ConversationRequest, HostedTool, SourceOccurrence, SourceOccurrenceId, ToolCall, rs};

async fn server(status: axum::http::StatusCode) -> (String, mpsc::UnboundedReceiver<Bytes>, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let app = Router::new().route("/{*path}", post(move |body: Bytes| {
        let tx = tx.clone();
        async move {
            tx.send(body).unwrap();
            (status, axum::Json(serde_json::json!({"error":{"message":"fixture failure","type":"invalid_request_error"}})))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    (url, rx, task)
}

fn request() -> ConversationRequest {
    let reasoning = rs::ReasoningItem {
        id: "reasoning-fixture".into(), summary: vec![],
        content: Some(vec![rs::ReasoningTextContent { text: "reasoning body".into() }]),
        encrypted_content: None, status: None,
    };
    let mut items = vec![
        ConversationItem::user("sensitive request body"),
        ConversationItem::Reasoning(reasoning.into()),
        ConversationItem::assistant_tool_calls(vec![ToolCall {
            id: "call-fixture".into(), name: "tool-fixture".into(), arguments: "invalid JSON".into(),
        }]),
        ConversationItem::tool_result("call-fixture", "result"),
        ConversationItem::user("continue"),
    ];
    for (index, item) in items.iter_mut().enumerate() {
        let mut id = [0; 16];
        id[0] = u8::try_from(index).unwrap();
        *item.source_mut() = SourceOccurrence::Admitted { id: SourceOccurrenceId(id) };
    }
    ConversationRequest { items, json_schema: Some(serde_json::json!({"type":"object","properties":{}})),
        hosted_tools: vec![HostedTool::WebSearch { options: None }], ..Default::default() }
}

fn assert_no_metadata(value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                assert_ne!(key, "source");
                assert_ne!(key, "authorization");
                assert!(!key.starts_with("x_grok_"));
                assert_no_metadata(value);
            }
        }
        serde_json::Value::Array(values) => values.iter().for_each(assert_no_metadata),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) | serde_json::Value::String(_) => {}
    }
}

#[tokio::test]
async fn final_capture_equals_http_body_after_each_provider_serializer() {
    for backend in [ApiBackend::ChatCompletions, ApiBackend::Responses, ApiBackend::Messages] {
        let (url, mut bodies, task) = server(axum::http::StatusCode::BAD_REQUEST).await;
        let config = SamplerConfig { base_url: url, model: "fixture-model".into(), api_backend: backend.clone(),
            api_key: Some(["fixture", "auth"].join("-")), stream_tool_calls: true,
            extra_response_includes: vec!["preview.additional.include".into()], ..Default::default() };
        let (preview_events, mut preview_rx) = mpsc::unbounded_channel();
        let sampler = crate::SamplerActor::spawn(config.clone(), crate::RetryPolicy::default(), preview_events);
        let preview = sampler.preview_request(request()).await.unwrap();
        assert_eq!(sampler.active_count().await, 0);
        assert!(matches!(preview_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        assert!(!format!("{preview:?}").contains("sensitive request body"));
        let (events, mut rx) = mpsc::unbounded_channel();
        let request = request();
        let capture = CaptureAttempt {
            request_id: crate::RequestId::from("fixture"), attempt: RequestAttemptId(1), backend: backend.clone(),
            membership: request.projected_membership().into(), cache: Arc::new(RequestBodyCache::default()), events,
        };
        let client = SamplingClient::new(config).unwrap().with_capture(capture);
        let result = match backend {
            ApiBackend::ChatCompletions => client.conversation_stream(request).await.map(|_| ()),
            ApiBackend::Responses => client.conversation_stream_responses(request).await.map(|_| ()),
            ApiBackend::Messages => client.conversation_stream_messages(request).await.map(|_| ()),
        };
        assert!(result.is_err());
        let actual = bodies.recv().await.unwrap();
        let SamplingEvent::LocalRequest { evidence: LocalRequestEvidence::Prepared { body, membership, .. }, .. } = rx.recv().await.unwrap() else {
            panic!("serializer evidence must precede dispatch");
        };
        assert_eq!(body.sensitive_local_bytes(), actual.as_ref());
        assert_eq!(preview.body.sensitive_local_bytes(), actual.as_ref());
        assert!(matches!(preview_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        assert!(!String::from_utf8(actual.to_vec()).unwrap().contains(&["fixture", "auth"].join("-")));
        assert_eq!(membership.len(), 5);
        assert!(!format!("{body:?}").contains("sensitive request body"));
        assert!(!format!("{client:?}").contains(&["fixture", "auth"].join("-")));
        let value: serde_json::Value = serde_json::from_slice(&actual).unwrap();
        assert_no_metadata(&value);
        assert_eq!(value["stream"], true);
        match backend {
            ApiBackend::Responses => {
                assert_eq!(value["stream_tool_calls"], true);
                assert!(value["include"].as_array().unwrap().iter().any(|include| include == "preview.additional.include"));
                assert!(value["tools"].as_array().unwrap().iter().any(|tool| tool["type"] == "web_search"));
                let reasoning = value["input"].as_array().unwrap().iter().find(|item| item["type"] == "reasoning").unwrap();
                assert_eq!(reasoning["content"][0]["type"], "reasoning_text");
                assert_eq!(value["text"]["format"]["type"], "json_schema");
            }
            ApiBackend::ChatCompletions => {
                let assistant = value["messages"].as_array().unwrap().iter().find(|item| item["tool_calls"].is_array()).unwrap();
                assert_eq!(assistant["tool_calls"][0]["function"]["arguments"], "{}");
            }
            ApiBackend::Messages => {}
        }
        assert!(matches!(rx.recv().await.unwrap(), SamplingEvent::LocalRequest { evidence: LocalRequestEvidence::DispatchStarted, .. }));
        assert!(matches!(rx.recv().await.unwrap(), SamplingEvent::LocalRequest { evidence: LocalRequestEvidence::ResponseHeaders { status: 400 }, .. }));
        task.abort();
    }
}

#[tokio::test]
async fn real_retry_loop_shares_body_but_retains_failed_attempts() {
    let (url, _bodies, task) = server(axum::http::StatusCode::SERVICE_UNAVAILABLE).await;
    let config = SamplerConfig { base_url: url, model: "fixture-model".into(), max_retries: Some(1), ..Default::default() };
    let (tx, mut rx) = mpsc::unbounded_channel();
    crate::actor::request_task::run_request_task(
        crate::RequestId::from("retry-fixture"), ConversationRequest::from_items(vec![ConversationItem::user("retry body")]),
        config, crate::RetryPolicy::default(), tx, tokio_util::sync::CancellationToken::new(), None,
    ).await;
    let mut prepared = Vec::new();
    let mut outcomes = Vec::new();
    while let Ok(event) = rx.try_recv() {
        match event {
            SamplingEvent::LocalRequest { attempt, evidence: LocalRequestEvidence::Prepared { body, .. }, .. } => prepared.push((attempt, body)),
            SamplingEvent::LocalRequest { attempt, evidence: LocalRequestEvidence::Finished { outcome }, .. } => outcomes.push((attempt, outcome)),
            _ => {}
        }
    }
    assert_eq!(prepared.len(), 2);
    assert_eq!(prepared[0].0, RequestAttemptId(1));
    assert_eq!(prepared[1].0, RequestAttemptId(2));
    assert!(Arc::ptr_eq(&prepared[0].1, &prepared[1].1));
    assert_eq!(outcomes, [(RequestAttemptId(1), LocalAttemptOutcome::InitializationFailed), (RequestAttemptId(2), LocalAttemptOutcome::InitializationFailed)]);
    task.abort();
}

#[tokio::test]
async fn preparing_a_body_does_not_claim_dispatch_or_acceptance() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let capture = CaptureAttempt {
        request_id: crate::RequestId::from("prepared-only"), attempt: RequestAttemptId(1), backend: ApiBackend::ChatCompletions,
        membership: Vec::new().into(), cache: Arc::new(RequestBodyCache::default()), events: tx,
    };
    let client = SamplingClient::new(SamplerConfig { base_url: "http://127.0.0.1:1/v1".into(), model: "fixture".into(), ..Default::default() }).unwrap().with_capture(capture);
    let payload = serde_json::json!({"messages":[{"role":"user","content":"prepared"}]});
    let builder = client.http.post("http://127.0.0.1:1/v1/chat/completions");
    let built = client.build_json_request(builder, crate::request_body::json_body(&payload).unwrap()).await.unwrap();
    let SamplingEvent::LocalRequest { evidence: LocalRequestEvidence::Prepared { body, .. }, .. } = rx.recv().await.unwrap() else { panic!("expected prepared evidence"); };
    assert_eq!(built.body().unwrap().as_bytes().unwrap(), body.sensitive_local_bytes());
    assert!(matches!(rx.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
}

#[tokio::test]
async fn compressed_http_body_decodes_to_the_exact_captured_json() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let capture = CaptureAttempt {
        request_id: crate::RequestId::from("compressed"), attempt: RequestAttemptId(1), backend: ApiBackend::ChatCompletions,
        membership: Vec::new().into(), cache: Arc::new(RequestBodyCache::default()), events: tx,
    };
    let client = SamplingClient::new(SamplerConfig {
        base_url: "http://127.0.0.1:1/v1".into(), model: "fixture".into(), request_compression: RequestCompression::Zstd, ..Default::default()
    }).unwrap().with_capture(capture);
    let payload = serde_json::json!({"content": "large-body".repeat(crate::request_compression::MIN_COMPRESS_BYTES)});
    let built = client.build_json_request(client.http.post("http://127.0.0.1:1/v1/chat/completions"), crate::request_body::json_body(&payload).unwrap()).await.unwrap();
    assert_eq!(built.headers().get(CONTENT_ENCODING).unwrap(), "zstd");
    let SamplingEvent::LocalRequest { evidence: LocalRequestEvidence::Prepared { body, .. }, .. } = rx.recv().await.unwrap() else { panic!("expected prepared evidence"); };
    let decoded = zstd::decode_all(built.body().unwrap().as_bytes().unwrap()).unwrap();
    assert_eq!(decoded, body.sensitive_local_bytes());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&decoded).unwrap(), payload);
}

#[tokio::test]
async fn local_body_debug_is_opaque_and_publication_preserves_redaction_evidence() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let capture = CaptureAttempt {
        request_id: crate::RequestId::from("privacy"), attempt: RequestAttemptId(1), backend: ApiBackend::ChatCompletions,
        membership: Vec::new().into(), cache: Arc::new(RequestBodyCache::default()), events: tx,
    };
    let sensitive = ["sk-", &"q".repeat(32)].concat();
    let bytes = serde_json::to_vec(&serde_json::json!({"content": sensitive})).unwrap();
    capture.prepared(&bytes);
    let event = rx.recv().await.unwrap();
    assert!(!format!("{event:?}").contains(&sensitive));
    let SamplingEvent::LocalRequest { evidence: LocalRequestEvidence::Prepared { body, .. }, .. } = event else { panic!("expected capture"); };
    assert_eq!(body.sensitive_local_bytes(), bytes);
    let publication = body.publication_view().unwrap();
    assert!(matches!(publication.representation, PayloadRepresentation::Redacted { .. }));
    assert!(!String::from_utf8(publication.bytes.to_vec()).unwrap().contains(&sensitive));
}
