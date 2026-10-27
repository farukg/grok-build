use std::sync::Arc;

use agent_client_protocol as acp;
use xai_acp_lib::AcpAgentMessage;
use xai_grok_shell::session::request_capture::{RequestSnapshot, RequestSnapshotId};
use xai_grok_shell::session::storage::request_capture::{BodySelection, SnapshotRead};

use super::query;
use crate::app::actions::{RequestSnapshotQuery, RequestSnapshotTransportError, TaskResult};
use crate::app::agent::AgentId;
use crate::views::request_viewer::{RequestBodyContent, RequestBodyKey};

const SESSION: &str = "11111111-1111-1111-1111-111111111111";
const REQUEST: &str = "22222222-2222-2222-2222-222222222222";

fn snapshot(turn: u64) -> RequestSnapshot {
    serde_json::from_value(serde_json::json!({
        "origin": {
            "session_id": SESSION, "request": REQUEST, "turn": turn, "generation": 1,
            "prompt_id": null, "conversation_id": null, "logical_request_id": null,
        },
        "observations": [{
            "attempt": 2,
            "observation": {
                "event": "prepared", "backend": "responses", "membership": [],
                "body": {
                    "digest": vec![7u8; 32], "byte_len": 16,
                    "availability": { "state": "unknown" },
                },
                "source_durability": "unconfirmed_independent_history_channel",
            },
        }],
    })).unwrap()
}

fn response(value: serde_json::Value) -> acp::ExtResponse {
    acp::ExtResponse::new(Arc::from(serde_json::value::to_raw_value(&value).unwrap()))
}

#[tokio::test]
async fn latest_uses_one_tail_read_for_empty_exact_page_and_large_histories() {
    for count in [0u64, 100, 200, 6400, 6401, 12000] {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let read = query(AgentId(0), acp::SessionId::new(SESSION), RequestSnapshotQuery::Latest, 17, &tx);
        let server = async {
            let Some(AcpAgentMessage::ExtMethod(args)) = rx.recv().await else { panic!("capture RPC"); };
            assert_eq!(args.request.method.as_ref(), "x.ai/session/request_snapshot");
            assert_eq!(serde_json::from_str::<serde_json::Value>(args.request.params.get()).unwrap(),
                serde_json::json!({ "sessionId": SESSION, "action": "latest" }));
            let reply = if count == 0 {
                serde_json::json!({ "result": "absent" })
            } else {
                serde_json::json!({ "result": "snapshot", "snapshot": snapshot(count) })
            };
            args.response_tx.send(Ok(response(reply))).unwrap();
        };
        let (result, ()) = tokio::join!(read, server);
        let TaskResult::RequestSnapshotLoaded { session_id, generation, result, .. } = result
            else { panic!("latest result"); };
        assert_eq!(session_id.0.as_ref(), SESSION);
        assert_eq!(generation, 17);
        if count == 0 {
            assert!(result.unwrap().is_none());
        } else {
            let snapshot = result.unwrap().unwrap();
            assert_eq!(snapshot.origin.turn, count);
            assert_eq!(snapshot.origin.request.0.to_string(), REQUEST);
        }
        assert!(rx.try_recv().is_err(), "latest never scans pages or issues a second Get");
    }
}

#[tokio::test]
async fn body_read_selects_the_prepared_digest_and_rejects_every_foreign_protected_reference_axis() {
    let _theme = crate::theme::cache::pin_theme();
    for (session, request, attempt, digest) in [
        ("33333333-3333-3333-3333-333333333333", REQUEST, 2, [7u8; 32]),
        (SESSION, "33333333-3333-3333-3333-333333333333", 2, [7; 32]),
        (SESSION, REQUEST, 3, [7; 32]),
        (SESSION, REQUEST, 2, [8; 32]),
        (SESSION, REQUEST, 2, [7; 32]),
    ] {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let key = RequestBodyKey::from_snapshot(&snapshot(1)).unwrap();
        let read = query(AgentId(0), acp::SessionId::new(SESSION),
            RequestSnapshotQuery::Body { key: key.clone() }, 18, &tx);
        let server = async {
            let Some(AcpAgentMessage::ExtMethod(args)) = rx.recv().await else { panic!("body RPC"); };
            let request_body: xai_grok_shell::extensions::request_snapshot::RequestSnapshotRequest =
                serde_json::from_str(args.request.params.get()).unwrap();
            assert_eq!(request_body.session_id, SESSION);
            let SnapshotRead::Body { request: selected, attempt: selected_attempt, selection } = request_body.action
                else { panic!("body selection"); };
            assert_eq!(selected, RequestSnapshotId(uuid::Uuid::parse_str(REQUEST).unwrap()));
            assert_eq!(selected_attempt.0, 2);
            assert!(matches!(selection, BodySelection::Digest { digest } if digest == [7; 32]));
            args.response_tx.send(Ok(response(serde_json::json!({
                "result": "body",
                "body": {
                    "state": "available", "json": "{\"model\":\"grok\"}",
                    "representation": { "representation": "exact_local_json" },
                    "protected_local": {
                        "snapshot": { "sessionId": session, "request": request },
                        "attempt": attempt, "digest": digest,
                    },
                },
            })))).unwrap();
        };
        let (result, ()) = tokio::join!(read, server);
        let TaskResult::RequestBodyLoaded { key: answered, result, .. } = result
            else { panic!("body result"); };
        assert_eq!(answered, key);
        if session == SESSION && request == REQUEST && attempt == 2 && digest == [7; 32] {
            assert!(matches!(result, Ok(RequestBodyContent::Available(_))));
        } else {
            assert!(matches!(result, Err(RequestSnapshotTransportError::UnexpectedShape)));
        }
        assert!(rx.try_recv().is_err());
    }
}
