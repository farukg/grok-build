#![allow(dead_code)]
use agent_client_protocol::{self as acp, Agent as _};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::Notify;
use xai_grok_shell::session::delivery_receiver::{
    DeliveryProgress, DeliveryReceipt, DeliveryResult,
};

#[derive(Clone, Default)]
pub struct Recorder {
    events: Rc<RefCell<Vec<Value>>>,
    changed: Rc<Notify>,
}
impl Recorder {
    pub async fn wait(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(crate::acp_harness::RPC_TIMEOUT, async {
            loop {
                let changed = self.changed.notified();
                if let Some(event) = self
                    .events
                    .borrow()
                    .iter()
                    .find(|event| predicate(event))
                    .cloned()
                {
                    return event;
                }
                changed.await;
            }
        })
        .await
        .expect("expected runtime event")
    }
    pub async fn terminal(&self, id: &str) -> DeliveryReceipt {
        let value = self
            .wait(|value| {
                value["method"] == "x.ai/session/delivery_changed"
                    && value["params"]["deliveryId"] == id
                    && matches!(
                        value["params"]["progress"]["state"].as_str(),
                        Some("succeeded" | "failed" | "cancelled" | "interrupted")
                    )
            })
            .await;
        serde_json::from_value(value["params"].clone()).expect("native receipt hint")
    }
    pub async fn child_finished(&self, id: &str) {
        self.wait(|event| {
            event["params"]["update"]["sessionUpdate"] == "subagent_finished"
                && event["params"]["update"]["subagent_id"] == id
        })
        .await;
    }
    pub async fn recovered_turn(&self) {
        self.wait(|event| {
            event["params"]["update"]["sessionUpdate"] == "turn_completed"
                && event["params"]["update"]["prompt_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("subagent-recovered-"))
        })
        .await;
    }
}
#[async_trait::async_trait(?Send)]
impl acp::Client for Recorder {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        Ok(acp::RequestPermissionResponse::new(
            crate::acp_harness::allow_once(&args),
        ))
    }
    async fn session_notification(&self, _args: acp::SessionNotification) -> acp::Result<()> {
        Ok(())
    }
    async fn ext_notification(&self, args: acp::ExtNotification) -> acp::Result<()> {
        let params: Value =
            serde_json::from_str(args.params.get()).expect("native notification JSON");
        self.events
            .borrow_mut()
            .push(json!({"method": args.method, "params": params}));
        self.changed.notify_one();
        Ok(())
    }
}

pub async fn delivery(
    conn: &acp::ClientSideConnection,
    session: &acp::SessionId,
    id: &str,
    payload: &str,
    action: &str,
) -> DeliveryResult {
    let value = crate::acp_harness::ext_method(conn, "x.ai/session/delivery", json!({
        "sessionId": session.to_string(), "deliveryId": id, "payload": payload, "action": action,
    })).await;
    serde_json::from_value(value).expect("exact receiver wire result")
}
pub fn admitted(result: DeliveryResult) -> DeliveryReceipt {
    match result {
        DeliveryResult::Admitted { receipt } => receipt,
        other => panic!("expected native durable receipt: {other:?}"),
    }
}
pub async fn seed_and_load(
    conn: &acp::ClientSideConnection,
    cwd: &std::path::Path,
    id: &str,
) -> acp::SessionId {
    use xai_grok_shell::session::storage::{JsonlStorageAdapter, StorageAdapter};
    let id = acp::SessionId::new(id.to_owned());
    let info = xai_grok_shell::session::info::Info {
        id: id.clone(),
        cwd: cwd.to_string_lossy().into_owned(),
    };
    JsonlStorageAdapter::new()
        .init_session(&info, acp::ModelId::new("test-model"))
        .await
        .expect("fixture initialized by actual storage owner");
    conn.load_session(acp::LoadSessionRequest::new(id.clone(), cwd.to_path_buf()))
        .await
        .expect("real parent load");
    id
}
pub async fn history(
    cwd: &std::path::Path,
    id: &acp::SessionId,
) -> Vec<xai_grok_shell::sampling::ConversationItem> {
    use xai_grok_shell::session::storage::{JsonlStorageAdapter, StorageAdapter};
    JsonlStorageAdapter::new()
        .load_session(&xai_grok_shell::session::info::Info {
            id: id.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
        })
        .await
        .expect("authoritative history")
        .chat_history
}
pub fn matching_history_metadata(
    items: &[xai_grok_shell::sampling::ConversationItem],
    marker: &str,
) -> impl std::fmt::Debug {
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            use sha2::{Digest, Sha256};
            let payload = serde_json::to_value(item).expect("canonical history payload");
            let serialized = payload.to_string();
            if !serialized.contains(marker) {
                return None;
            }
            let text = item.text_content();
            Some((
                index,
                payload["type"]
                    .as_str()
                    .expect("canonical history variant")
                    .to_owned(),
                item.role(),
                item.synthetic_reason().cloned(),
                item.source(),
                text.matches(marker).count(),
                serialized.matches(marker).count(),
                format!("{:x}", Sha256::digest(text.as_bytes())),
                format!("{:x}", Sha256::digest(serialized.as_bytes())),
            ))
        })
        .collect::<Vec<_>>()
}

#[derive(Clone, Copy)]
pub enum CompletionHistoryKind {
    ToolResult,
    SubagentCompletion,
}

pub fn assert_completion_projection_once(
    cwd: &std::path::Path,
    session: &acp::SessionId,
    child: &str,
    items: &[xai_grok_shell::sampling::ConversationItem],
    marker: &str,
    kind: CompletionHistoryKind,
) {
    use xai_chat_state::completion::CompletionProjection;
    use xai_grok_shell::sampling::{ConversationItem, SyntheticReason};
    let info = xai_grok_shell::session::info::Info {
        id: session.clone(),
        cwd: cwd.to_string_lossy().into_owned(),
    };
    let history = std::fs::read_to_string(
        xai_grok_shell::session::persistence::session_dir(&info).join("chat_history.jsonl"),
    )
    .expect("canonical completion history");
    let projections = history
        .lines()
        .filter_map(|line| {
            let payload: Value = serde_json::from_str(line).expect("canonical history JSON");
            payload.get("_subagent_completion")?;
            let projection: CompletionProjection =
                serde_json::from_value(payload).expect("canonical completion projection");
            let original = projection.receipts().any(|receipt| {
                receipt
                    .completion
                    .subagent()
                    .is_some_and(|id| id.subagent_id == child)
            });
            original.then_some(projection)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        projections.len(),
        1,
        "one identity-bound original completion projection; matches={:?}",
        matching_history_metadata(items, marker)
    );
    let projection = &projections[0];
    assert_eq!(
        projection
            .receipts()
            .filter(|receipt| {
                receipt
                    .completion
                    .subagent()
                    .is_some_and(|id| id.subagent_id == child)
            })
            .count(),
        1,
        "one receipt for the original child"
    );
    let expected_kind = match kind {
        CompletionHistoryKind::ToolResult => {
            matches!(&projection.item, ConversationItem::ToolResult(_))
        }
        CompletionHistoryKind::SubagentCompletion => matches!(
            &projection.item,
            ConversationItem::User(user)
                if user.synthetic_reason == SyntheticReason::SubagentCompleted
        ),
    };
    assert!(
        expected_kind,
        "original completion retains its canonical variant"
    );
    assert!(!projection.item.source().is_unadmitted());
    let retained = items
        .iter()
        .filter(|item| item.source() == projection.item.source())
        .collect::<Vec<_>>();
    assert_eq!(
        retained.len(),
        1,
        "original completion source survives history loading"
    );
    assert!(
        serde_json::to_value(retained[0]).expect("loaded completion payload")
            == serde_json::to_value(&projection.item).expect("persisted completion payload"),
        "loaded completion preserves the canonical payload"
    );
    assert_eq!(
        projection.item.text_content().matches(marker).count(),
        1,
        "the original projection must not repeat its payload; matches={:?}",
        matching_history_metadata(items, marker)
    );
}

pub fn assert_goal_final_report(
    items: &[xai_grok_shell::sampling::ConversationItem],
    report: &str,
) {
    use xai_grok_shell::sampling::ConversationItem;
    let goal_calls = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            ConversationItem::Assistant(assistant) => Some((index, &assistant.tool_calls)),
            ConversationItem::System(_)
            | ConversationItem::User(_)
            | ConversationItem::ToolResult(_)
            | ConversationItem::BackendToolCall(_)
            | ConversationItem::Reasoning(_) => None,
        })
        .flat_map(|(index, calls)| calls.iter().map(move |call| (index, call)))
        .filter(|(_, call)| call.name == "update_goal")
        .collect::<Vec<_>>();
    assert_eq!(goal_calls.len(), 1, "one actual update_goal call");
    let (goal_index, goal_call) = goal_calls[0];
    let arguments: Value = serde_json::from_str(&goal_call.arguments).expect("goal tool arguments");
    assert!(
        arguments["completed"] == true,
        "the actual goal call completes the goal"
    );
    let acknowledgements = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(item, ConversationItem::ToolResult(_))
                && item.text_content().contains("Goal marked complete.")
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let reports = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(item, ConversationItem::Assistant(assistant)
                if assistant.content.as_ref() == report && assistant.tool_calls.is_empty())
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert_eq!(
        acknowledgements.len(),
        1,
        "one actual goal tool acknowledgement"
    );
    assert_eq!(reports.len(), 1, "one exact final assistant report");
    assert!(goal_index < acknowledgements[0]);
    assert!(
        matches!(
            &items[acknowledgements[0]],
            ConversationItem::ToolResult(result) if result.tool_call_id == goal_call.id.as_ref()
        ),
        "goal acknowledgement belongs to the actual update_goal call"
    );
    assert!(acknowledgements[0] < reports[0]);
}

pub fn main_requests(server: &xai_grok_test_support::MockInferenceServer, id: &str) -> Vec<Value> {
    server
        .requests()
        .into_iter()
        .filter(|request| request.header("x-grok-session-id") == Some(id))
        .filter(|request| request.header("x-grok-turn-idx").is_some())
        .map(|request| {
            request
                .body
                .expect("foreground request must retain its JSON payload")
        })
        .collect()
}
pub fn assert_succeeded(receipt: &DeliveryReceipt) {
    assert_eq!(receipt.progress, DeliveryProgress::Succeeded);
}

use xai_grok_test_support::{ScriptedResponse, SseEvent};
pub fn text_reply(text: &str) -> ScriptedResponse {
    ScriptedResponse::sse(vec![
        SseEvent::data(json!({"id":"receiver-reply","object":"chat.completion.chunk","created":1234567890,"model":"test-model","choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":null}]}).to_string()),
        SseEvent::data(json!({"id":"receiver-reply","object":"chat.completion.chunk","created":1234567890,"model":"test-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}).to_string()),
        SseEvent::data("[DONE]"),
    ])
}
pub fn tool_reply(name: &str, arguments: serde_json::Value) -> ScriptedResponse {
    ScriptedResponse::sse(vec![
        SseEvent::data(json!({"id":"receiver-tool","object":"chat.completion.chunk","created":1234567890,"model":"test-model","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"receiver-tool-call","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":null}]}).to_string()),
        SseEvent::data(json!({"id":"receiver-tool","object":"chat.completion.chunk","created":1234567890,"model":"test-model","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}).to_string()),
        SseEvent::data("[DONE]"),
    ])
}
