//! Replays records earlier builds wrote and asserts on what reaches the client.

use agent_client_protocol as acp;
use serde_json::Value;

use super::MvpAgent;
use crate::tools::task_completed_frame::FRAME_MAX_BYTES;
use xai_acp_lib::{AcpAgentGatewaySender as GatewaySender, AcpClientMessage};

/// The record is built from the real notification type.
/// Renaming a field the refit looks up by name then fails this test rather than silently putting oversized lines back on the wire.
#[tokio::test]
async fn replay_shrinks_an_oversized_completion() {
    let (agent, mut rx) = build_agent_with_gateway();
    let line = replay_line(&recorded_completion("Z".repeat(2 * 1024 * 1024)));

    agent.forward_raw_replay_line(
        &line,
        /*persist_data*/ None,
        /*target_client_id*/ None,
        /*mark_replay*/ false,
        &mut crate::session::storage::ReplayToolCollapser::new(),
        None,
    );

    let params = next_ext_notification_params(&mut rx).expect("the record must still be sent");
    assert!(
        params.len() <= FRAME_MAX_BYTES,
        "replayed {} bytes",
        params.len()
    );
    assert!(
        params.contains("/tmp/bg-old.log"),
        "the log pointer is kept"
    );
    assert!(
        params.contains("keep me"),
        "a field this build does not model must survive the refit"
    );
}

fn recorded_completion(output: String) -> Value {
    use crate::extensions::notification::{SessionNotification, SessionUpdate};

    let notification = SessionNotification {
        session_id: acp::SessionId::new("s"),
        update: SessionUpdate::TaskCompleted {
            task_snapshot: xai_grok_tools::types::TaskSnapshot {
                task_id: "bg-old".to_string(),
                command: "grep -r pattern .".to_string(),
                display_command: None,
                cwd: "/workspace".to_string(),
                start_time: std::time::SystemTime::now(),
                end_time: Some(std::time::SystemTime::now()),
                output,
                output_file: std::path::PathBuf::from("/tmp/bg-old.log"),
                truncated: false,
                output_total_bytes: 0,
                exit_code: Some(0),
                signal: None,
                completed: true,
                block_waited: false,
                explicitly_killed: false,
                kill_result_delivered: false,
                kind: Default::default(),
                owner_session_id: None,
                description: None,
                is_backgrounded: true,
            },
            will_wake: false,
        },
        meta: None,
    };
    let mut record = serde_json::to_value(&notification).expect("serialize");
    if let Some(snap) = record
        .get_mut("update")
        .and_then(|u| u.get_mut("task_snapshot"))
        .and_then(|s| s.as_object_mut())
    {
        snap.insert(
            "a_field_from_another_build".into(),
            Value::String("keep me".to_string()),
        );
    }
    record
}

fn replay_line(record: &Value) -> String {
    serde_json::json!({ "method": "_x.ai/session/update", "params": record }).to_string()
}

/// The branch a plain resume takes: `_meta` is added to the record, so the fit has to happen after that and not before.
#[tokio::test]
async fn a_marked_replay_is_fitted_after_its_metadata_is_added() {
    let (agent, mut rx) = build_agent_with_gateway();
    let line = replay_line(&recorded_completion("Z".repeat(2 * 1024 * 1024)));
    let persist = serde_json::json!({ "padding": "p".repeat(8 * 1024) });

    agent.forward_raw_replay_line(
        &line,
        Some(&persist),
        /*target_client_id*/ None,
        /*mark_replay*/ true,
        &mut crate::session::storage::ReplayToolCollapser::new(),
        None,
    );

    let params = next_ext_notification_params(&mut rx).expect("the record must still be sent");
    assert!(
        params.len() <= FRAME_MAX_BYTES,
        "replayed {} bytes once the metadata was added",
        params.len()
    );
    assert!(params.contains("isReplay"));
}

/// A completion nothing can shrink is dropped, because sending it is what closes the connection.
#[tokio::test]
async fn replay_drops_a_completion_nothing_can_shrink() {
    let (agent, mut rx) = build_agent_with_gateway();
    let mut record = recorded_completion(String::new());
    if let Some(snap) = record
        .get_mut("update")
        .and_then(|u| u.get_mut("task_snapshot"))
        .and_then(|s| s.as_object_mut())
    {
        snap.insert(
            "task_id".into(),
            Value::String("t".repeat(2 * FRAME_MAX_BYTES)),
        );
    }
    let line = replay_line(&record);

    agent.forward_raw_replay_line(
        &line,
        /*persist_data*/ None,
        /*target_client_id*/ None,
        /*mark_replay*/ false,
        &mut crate::session::storage::ReplayToolCollapser::new(),
        None,
    );

    assert!(next_ext_notification_params(&mut rx).is_none());
}

#[tokio::test]
async fn replay_forwards_records_within_the_limit_untouched() {
    let (agent, mut rx) = build_agent_with_gateway();
    let record = recorded_completion("hi\n".to_string());
    let line = replay_line(&record);

    agent.forward_raw_replay_line(
        &line,
        /*persist_data*/ None,
        /*target_client_id*/ None,
        /*mark_replay*/ false,
        &mut crate::session::storage::ReplayToolCollapser::new(),
        None,
    );

    let params = next_ext_notification_params(&mut rx).expect("forwarded");
    let mut expected = record;
    expected["_meta"] = serde_json::json!({});
    crate::extensions::current_actions::stamp_delivery(
        expected["_meta"].as_object_mut().unwrap(),
        &crate::extensions::current_actions::DeliveryRole::Unavailable,
    )
    .unwrap();
    assert_eq!(params, serde_json::to_string(&expected).unwrap());
}

#[tokio::test]
async fn replay_leaves_other_oversized_records_alone() {
    let (agent, mut rx) = build_agent_with_gateway();

    let recorded = format!(
        r#"{{"sessionId":"s","update":{{"sessionUpdate":"subagent_spawned","subagent_id":"{}"}}}}"#,
        "x".repeat(64 * 1024)
    );
    let line = format!(r#"{{"method":"_x.ai/session/update","params":{recorded}}}"#);
    agent.forward_raw_replay_line(
        &line,
        /*persist_data*/ None,
        /*target_client_id*/ None,
        /*mark_replay*/ false,
        &mut crate::session::storage::ReplayToolCollapser::new(),
        None,
    );

    let params = next_ext_notification_params(&mut rx).expect("forwarded");
    let mut expected: Value = serde_json::from_str(&recorded).unwrap();
    expected["_meta"] = serde_json::json!({});
    crate::extensions::current_actions::stamp_delivery(
        expected["_meta"].as_object_mut().unwrap(),
        &crate::extensions::current_actions::DeliveryRole::Unavailable,
    )
    .unwrap();
    assert_eq!(params, serde_json::to_string(&expected).unwrap());
}

#[tokio::test]
async fn parent_replay_does_not_emit_meta_less_unfinished_tool_call() {
    let (agent, mut rx) = build_agent_with_gateway();
    let mut collapser = crate::session::storage::ReplayToolCollapser::new();
    let tool = r#"{"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"tool_call","toolCallId":"t1","title":"bash","status":"pending"},"_meta":{"eventId":"s-1"}}}"#;
    let start = r#"{"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"tool_call_update","toolCallId":"t1","title":"ls"},"_meta":{"eventId":"s-2"}}}"#;
    agent.forward_raw_replay_line(tool, None, None, false, &mut collapser, None);
    agent.forward_raw_replay_line(start, None, None, false, &mut collapser, None);
    assert_eq!(collapser.pending_len(), 1);
    assert!(
        rx.try_recv().is_err(),
        "parent must not synthesize a meta-less ToolCall at EOF"
    );
}

async fn unbound_replay_erases_archived_authority(cursor: Option<&str>) {
    use crate::extensions::current_actions::{
        DeliveryRole, PublicationContext, delivery_role, stamp_delivery,
    };
    use crate::session::current_actions::SnapshotBinding;
    let (agent, mut rx) = build_agent_with_gateway();
    let dir = tempfile::tempdir().unwrap();
    let cwd = xai_grok_paths::AbsPathBuf::new(dir.path().to_path_buf()).unwrap();
    let archived = DeliveryRole::Live {
        context: PublicationContext {
            binding: SnapshotBinding {
                session_id: "s".into(),
                binding_epoch: 7,
                load_generation: 9,
            },
            incarnation: uuid::Uuid::new_v4(),
        },
    };
    let native = acp::SessionNotification::new(
        acp::SessionId::new("s"),
        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::from(
            "original native content",
        ))),
    );
    let mut records = vec![
        serde_json::to_value(&native).unwrap(),
        serde_json::to_value(&native).unwrap(),
        recorded_completion("original xAI output".into()),
    ];
    let mut lines = Vec::new();
    for (index, record) in records.iter_mut().enumerate() {
        record["_meta"] = serde_json::json!({ "eventId": format!("s-{}", index + 1), "promptId": "original-prompt", "agentTimestampMs": 123 });
        stamp_delivery(record["_meta"].as_object_mut().unwrap(), &archived).unwrap();
        let method = if index == 2 {
            "_x.ai/session/update"
        } else {
            "session/update"
        };
        lines.push(
            serde_json::json!({ "timestamp": index, "method": method, "params": record })
                .to_string(),
        );
    }
    let path = dir.path().join("updates.jsonl");
    std::fs::write(&path, lines.join("\n")).unwrap();
    let updates_path = Some(path);
    let session_id = acp::SessionId::new("s");
    let replay = agent.replay_session_updates(
        &session_id,
        &cwd,
        &updates_path,
        None,
        None,
        cursor,
        false,
        None,
    );
    let expected = if cursor.is_some() {
        &records[1..]
    } else {
        &records[..]
    };
    let receive = async {
        for original in expected {
            let delivered = match rx.recv().await.unwrap() {
                AcpClientMessage::SessionNotification(record) => {
                    let value = serde_json::to_value(&record.request).unwrap();
                    record.response_tx.send(Ok(())).unwrap();
                    value
                }
                AcpClientMessage::ExtNotification(record) => {
                    assert_eq!(record.request.method.as_ref(), "x.ai/session/update");
                    let value = serde_json::from_str::<Value>(record.request.params.get()).unwrap();
                    record.response_tx.send(Ok(())).unwrap();
                    value
                }
                _ => panic!("actual native replay notifications only"),
            };
            assert_eq!(
                delivery_role(delivered["_meta"].as_object()).unwrap(),
                DeliveryRole::Unavailable
            );
            let mut expected = original.clone();
            stamp_delivery(
                expected["_meta"].as_object_mut().unwrap(),
                &DeliveryRole::Unavailable,
            )
            .unwrap();
            if cursor.is_none() {
                expected["_meta"]["isReplay"] = true.into();
            }
            assert_eq!(
                delivered, expected,
                "replay changes authority, not original payload or identities"
            );
        }
    };
    let (result, ()) = tokio::join!(replay, receive);
    result.unwrap();
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn unbound_full_replay_keeps_original_records_without_archived_live_authority() {
    unbound_replay_erases_archived_authority(None).await;
}

#[tokio::test]
async fn unbound_cursor_replay_keeps_original_records_without_archived_live_authority() {
    unbound_replay_erases_archived_authority(Some("s-1")).await;
}

/// Stale-task reconciliation on a cold load builds its completion from a recorded command of any size, so its line is measured like the rest.
#[tokio::test]
async fn a_stale_task_completion_is_frame_bounded() {
    let (agent, mut rx) = build_agent_with_gateway();
    let dir = tempfile::tempdir().unwrap();
    let line = format!(
        r#"{{"timestamp":1,"method":"_x.ai/session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"task_backgrounded","task_id":"stale-1","command":"{}","cwd":"/tmp"}}}}}}"#,
        "c".repeat(64 * 1024)
    );
    let path = dir.path().join("updates.jsonl");
    std::fs::write(&path, line).unwrap();

    agent.reconcile_stale_background_tasks(&acp::SessionId::new("s"), &Some(path));

    let params = next_ext_notification_params(&mut rx).expect("the stale task must be reported");
    assert!(
        params.len() <= FRAME_MAX_BYTES,
        "reconciliation emitted {} bytes",
        params.len()
    );
    assert!(params.contains("session_restart"));
}

#[tokio::test]
async fn stale_task_reconcile_keeps_incrementals_without_list() {
    let (agent, mut rx) = build_agent_with_gateway();
    let dir = tempfile::tempdir().unwrap();
    let line = r#"{"timestamp":1,"method":"_x.ai/session/update","params":{"sessionId":"s","update":{"sessionUpdate":"task_backgrounded","task_id":"stale-1","command":"sleep 1","cwd":"/tmp"}}}"#;
    let path = dir.path().join("updates.jsonl");
    std::fs::write(&path, line).unwrap();

    agent.reconcile_stale_background_tasks(&acp::SessionId::new("s"), &Some(path));

    let notifs = drain_ext_notifications(&mut rx);
    assert!(
        notifs.iter().any(|(method, params)| {
            method == "x.ai/task_completed" && params.contains("session_restart")
        }),
        "incremental completion must stay: {notifs:?}"
    );
    assert!(
        !notifs.iter().any(|(method, params)| {
            method == "x.ai/session_notification"
                && params.contains("\"sessionUpdate\":\"background_tasks\"")
        }),
        "reconcile is not the last-wins list writer: {notifs:?}"
    );
}

#[tokio::test]
async fn cold_load_with_no_orphans_emits_no_reconcile_notifications() {
    let (agent, mut rx) = build_agent_with_gateway();
    let dir = tempfile::tempdir().unwrap();
    let line = r#"{"timestamp":1,"method":"_x.ai/session/update","params":{"sessionId":"s","update":{"sessionUpdate":"task_backgrounded","task_id":"done-1","command":"sleep 1","cwd":"/tmp"}}}
{"timestamp":2,"method":"_x.ai/session/update","params":{"sessionId":"s","update":{"sessionUpdate":"task_completed","task_snapshot":{"task_id":"done-1"}}}}"#;
    let path = dir.path().join("updates.jsonl");
    std::fs::write(&path, line).unwrap();

    agent.reconcile_stale_background_tasks(&acp::SessionId::new("s"), &Some(path));

    let notifs = drain_ext_notifications(&mut rx);
    assert!(
        notifs.is_empty(),
        "paired tasks must not emit reconcile traffic: {notifs:?}"
    );
}

fn build_agent_with_gateway() -> (
    MvpAgent,
    tokio::sync::mpsc::UnboundedReceiver<AcpClientMessage>,
) {
    use crate::agent::config::Config as AgentConfig;
    use xai_grok_login::{AuthManager, GrokComConfig};

    let temp_dir = tempfile::tempdir().unwrap();
    let auth_manager =
        std::sync::Arc::new(AuthManager::new(temp_dir.path(), GrokComConfig::default()));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let agent = MvpAgent::new(
        GatewaySender::new(tx),
        &AgentConfig::default(),
        auth_manager,
        None,
        None,
    )
    .expect("valid test config");
    (agent, rx)
}

fn next_ext_notification_params(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<AcpClientMessage>,
) -> Option<String> {
    drain_ext_notifications(rx)
        .into_iter()
        .next()
        .map(|(_, params)| params)
}

fn drain_ext_notifications(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<AcpClientMessage>,
) -> Vec<(String, String)> {
    let mut notifs = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let AcpClientMessage::ExtNotification(args) = msg {
            notifs.push((
                args.request.method.as_ref().to_string(),
                args.request.params.get().to_string(),
            ));
            let _ = args.response_tx.send(Ok(()));
        }
    }
    notifs
}
