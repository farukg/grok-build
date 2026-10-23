#![allow(dead_code)]
use std::cell::RefCell;
use std::rc::Rc;
use agent_client_protocol::{self as acp, Agent as _};
use serde_json::{Value, json};
use tokio::sync::Notify;
use xai_grok_shell::session::delivery_receiver::{DeliveryProgress, DeliveryReceipt, DeliveryResult};

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
                if let Some(event) = self.events.borrow().iter().find(|event| predicate(event)).cloned() { return event; }
                changed.await;
            }
        }).await.expect("expected runtime event")
    }
    pub async fn terminal(&self, id: &str) -> DeliveryReceipt {
        let value = self.wait(|value| {
            value["method"] == "x.ai/session/delivery_changed" && value["params"]["deliveryId"] == id
                && matches!(value["params"]["progress"]["state"].as_str(), Some("succeeded" | "failed" | "cancelled" | "interrupted"))
        }).await;
        serde_json::from_value(value["params"].clone()).expect("native receipt hint")
    }
    pub async fn child_finished(&self, id: &str) {
        self.wait(|event| event["params"]["update"]["sessionUpdate"] == "subagent_finished"
            && event["params"]["update"]["subagent_id"] == id).await;
    }
    pub async fn recovered_turn(&self) {
        self.wait(|event| event["params"]["update"]["sessionUpdate"] == "turn_completed"
            && event["params"]["update"]["prompt_id"].as_str().is_some_and(|id| id.starts_with("subagent-recovered-"))).await;
    }
}
#[async_trait::async_trait(?Send)]
impl acp::Client for Recorder {
    async fn request_permission(&self, args: acp::RequestPermissionRequest) -> acp::Result<acp::RequestPermissionResponse> {
        Ok(acp::RequestPermissionResponse::new(crate::acp_harness::allow_once(&args)))
    }
    async fn session_notification(&self, _args: acp::SessionNotification) -> acp::Result<()> { Ok(()) }
    async fn ext_notification(&self, args: acp::ExtNotification) -> acp::Result<()> {
        let params: Value = serde_json::from_str(args.params.get()).expect("native notification JSON");
        self.events.borrow_mut().push(json!({"method": args.method, "params": params}));
        self.changed.notify_one();
        Ok(())
    }
}

pub async fn delivery(conn: &acp::ClientSideConnection, session: &acp::SessionId, id: &str, payload: &str, action: &str) -> DeliveryResult {
    let value = crate::acp_harness::ext_method(conn, "x.ai/session/delivery", json!({
        "sessionId": session.to_string(), "deliveryId": id, "payload": payload, "action": action,
    })).await;
    serde_json::from_value(value).expect("exact receiver wire result")
}
pub fn admitted(result: DeliveryResult) -> DeliveryReceipt {
    match result { DeliveryResult::Admitted { receipt } => receipt, other => panic!("expected native durable receipt: {other:?}") }
}
pub async fn seed_and_load(conn: &acp::ClientSideConnection, cwd: &std::path::Path, id: &str) -> acp::SessionId {
    use xai_grok_shell::session::storage::{JsonlStorageAdapter, StorageAdapter};
    let id = acp::SessionId::new(id.to_owned());
    let info = xai_grok_shell::session::info::Info { id: id.clone(), cwd: cwd.to_string_lossy().into_owned() };
    JsonlStorageAdapter::new().init_session(&info, acp::ModelId::new("test-model")).await.expect("fixture initialized by actual storage owner");
    conn.load_session(acp::LoadSessionRequest::new(id.clone(), cwd.to_path_buf())).await.expect("real parent load");
    id
}
pub async fn history(cwd: &std::path::Path, id: &acp::SessionId) -> Vec<xai_grok_shell::sampling::ConversationItem> {
    use xai_grok_shell::session::storage::{JsonlStorageAdapter, StorageAdapter};
    JsonlStorageAdapter::new().load_session(&xai_grok_shell::session::info::Info { id: id.clone(), cwd: cwd.to_string_lossy().into_owned() })
        .await.expect("authoritative history").chat_history
}
pub fn main_requests(server: &xai_grok_test_support::MockInferenceServer, id: &str) -> Vec<Value> {
    server.requests().into_iter().filter(|request| request.header("x-grok-session-id") == Some(id))
        .filter(|request| request.header("x-grok-turn-idx").is_some())
        .filter_map(|request| request.body).collect()
}
pub fn assert_succeeded(receipt: &DeliveryReceipt) { assert_eq!(receipt.progress, DeliveryProgress::Succeeded); }

use xai_grok_test_support::{ScriptedResponse, SseEvent};
pub fn text_reply(text: &str) -> ScriptedResponse {
    ScriptedResponse::sse(vec![
        SseEvent::data(json!({"id":"receiver-reply","object":"chat.completion.chunk","model":"test-model","choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":null}]}).to_string()),
        SseEvent::data(json!({"id":"receiver-reply","object":"chat.completion.chunk","model":"test-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}).to_string()),
        SseEvent::data("[DONE]"),
    ])
}
pub fn tool_reply(name: &str, arguments: serde_json::Value) -> ScriptedResponse {
    ScriptedResponse::sse(vec![
        SseEvent::data(json!({"id":"receiver-tool","object":"chat.completion.chunk","model":"test-model","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"receiver-tool-call","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":null}]}).to_string()),
        SseEvent::data(json!({"id":"receiver-tool","object":"chat.completion.chunk","model":"test-model","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}).to_string()),
        SseEvent::data("[DONE]"),
    ])
}

