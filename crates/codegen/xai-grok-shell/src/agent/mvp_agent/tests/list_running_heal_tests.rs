//! `list_running_subagents` must heal a live parent's stale `running` meta (tray / reconnect).
//! Deleting that hook leaves the tray showing a dead subagent as Responding for 10-12 hours.

use super::{build_minimal_agent_for_tests, make_live_session_handle};
use crate::agent::subagent::{LIVE_ORPHAN_RECONCILE_REASON, SubagentMeta, SubagentMetaStatus};
use crate::extensions::notification::SessionUpdate;
use crate::session::SessionCommand;
use agent_client_protocol as acp;
use xai_grok_tools::implementations::grok_build::task::types::{
    SubagentEvent, SubagentInspection, SubagentSnapshot, SubagentSnapshotStatus,
};

fn running_meta(id: &str, parent: &str) -> SubagentMeta {
    SubagentMeta {
        subagent_id: id.into(),
        attempt_id: None,
        parent_session_id: parent.into(),
        child_session_id: format!("child-{id}"),
        subagent_type: "explore".into(),
        description: "task".into(),
        prompt: "do work".into(),
        status: SubagentMetaStatus::Running,
        started_at: chrono::Utc::now(),
        completed_at: None,
        duration_ms: None,
        tool_calls: None,
        turns: None,
        error: None,
        context: crate::extensions::subagent_context::SubagentContext::Unreported,
        persona: None,
        resumed_from: None,
        child_cwd: Some("/workspace".into()),
        worktree_path: None,
        snapshot_ref: None,
        effective_model_id: None,
    }
}

fn write_meta(dir: &std::path::Path, meta: &SubagentMeta) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("meta.json"),
        serde_json::to_string_pretty(meta).unwrap(),
    )
    .unwrap();
}

fn running_inspection(id: &str, parent: &str) -> SubagentInspection {
    SubagentInspection {
        terminal: None,
        snapshot: SubagentSnapshot {
            subagent_id: id.to_string(),
            description: "task".to_string(),
            subagent_type: "explore".to_string(),
            status: SubagentSnapshotStatus::Running {
                turn_count: 1,
                tool_call_count: 0,
                tokens_used: 0,
                context_window_tokens: 0,
                context_usage_pct: 0,
                tools_used: Vec::new(),
                error_count: 0,
            },
            started_at_epoch_ms: 0,
            duration_ms: 50,
            persona: None,
        },
        parent_session_id: parent.to_string(),
        child_session_id: format!("child-{id}"),
        fork_parent_prompt_id: None,
        resumed_from: None,
    }
}

fn drain_cancelled_finishes(
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SessionCommand>,
    id: &str,
) -> usize {
    let mut count = 0;
    while let Ok(cmd) = cmd_rx.try_recv() {
        let SessionCommand::XaiSessionNotification { notification } = cmd else {
            continue;
        };
        let SessionUpdate::SubagentFinished {
            subagent_id,
            status,
            error,
            will_wake,
            ..
        } = notification.update
        else {
            continue;
        };
        if subagent_id != id {
            continue;
        }
        assert_eq!(status, "cancelled");
        assert_eq!(error.as_deref(), Some(LIVE_ORPHAN_RECONCILE_REASON));
        assert!(!will_wake);
        count += 1;
    }
    count
}

fn spawn_inspect_stub(
    event_rx: xai_grok_tools::implementations::grok_build::task::coordinator::SubagentCoordinatorReceiver,
    inspect: Option<SubagentInspection>,
) {
    let mut event_rx = event_rx.into_event_receiver();
    tokio::task::spawn_local(async move {
        while let Some(event) = event_rx.recv().await {
            if let SubagentEvent::Inspect(request) = event {
                let value = inspect
                    .as_ref()
                    .filter(|i| i.snapshot.subagent_id == request.subagent_id)
                    .cloned();
                let _ = request.respond_to.send(value);
            } else if let SubagentEvent::ListRunning(request) = event {
                let list = inspect
                    .as_ref()
                    .filter(|i| i.snapshot.is_running())
                    .cloned()
                    .into_iter()
                    .collect();
                let _ = request.respond_to.send(list);
            }
        }
    });
}

async fn live_session_with_running_meta(
    id: &str,
    inspect: Option<SubagentInspection>,
) -> (
    super::MvpAgent,
    String,
    std::path::PathBuf,
    tokio::sync::mpsc::UnboundedReceiver<SessionCommand>,
) {
    let agent = build_minimal_agent_for_tests();
    let event_rx = agent
        .subagent_event_rx
        .borrow_mut()
        .take()
        .expect("subagent event rx still available");
    spawn_inspect_stub(event_rx, inspect);

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sid = acp::SessionId::new(format!("list-running-{id}-{unique}"));
    let (handle, _cmd_tx, cmd_rx) = make_live_session_handle(&sid, None);
    let parent = sid.0.to_string();
    let session_dir = crate::session::persistence::session_dir(&handle.info);
    let sub_dir = session_dir.join("subagents").join(id);
    write_meta(&sub_dir, &running_meta(id, &parent));
    agent.insert_resident(&sid, handle);
    (agent, parent, sub_dir, cmd_rx)
}

#[tokio::test(flavor = "current_thread")]
async fn list_running_subagents_finalizes_orphan_on_live_session() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let id = "sa-list-orphan";
            let (agent, parent, sub_dir, mut cmd_rx) =
                live_session_with_running_meta(id, None).await;

            let listed = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                agent.list_running_subagents(&parent),
            )
            .await
            .expect("list_running heal must not hang")
            .expect("coordinator ACK");
            assert!(listed.is_empty());

            let reread: SubagentMeta =
                serde_json::from_str(&std::fs::read_to_string(sub_dir.join("meta.json")).unwrap())
                    .unwrap();
            assert_eq!(reread.status, SubagentMetaStatus::Cancelled);
            assert_eq!(drain_cancelled_finishes(&mut cmd_rx, id), 1);
            let _ = std::fs::remove_dir_all(sub_dir.parent().unwrap());
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn list_running_subagents_skips_live_coordinator_child() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let id = "sa-list-live";
            let (agent, parent, sub_dir, mut cmd_rx) =
                live_session_with_running_meta(id, Some(running_inspection(id, "parent"))).await;

            let listed = agent.list_running_subagents(&parent).await.expect("coordinator ACK");
            assert_eq!(listed.len(), 1);
            let Some(first) = listed.first() else {
                panic!("expected one running subagent: {listed:?}");
            };
            assert_eq!(first.snapshot.subagent_id, id);

            let reread: SubagentMeta =
                serde_json::from_str(&std::fs::read_to_string(sub_dir.join("meta.json")).unwrap())
                    .unwrap();
            assert_eq!(reread.status, SubagentMetaStatus::Running);
            assert_eq!(drain_cancelled_finishes(&mut cmd_rx, id), 0);
            let _ = std::fs::remove_dir_all(sub_dir.parent().unwrap());
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn list_running_lost_ack_preserves_running_history() {
    let local = tokio::task::LocalSet::new();
    local.run_until(async {
        let agent = build_minimal_agent_for_tests();
        let mut event_rx = agent.subagent_event_rx.borrow_mut().take().unwrap()
            .into_event_receiver();
        let receiver = tokio::task::spawn_local(async move {
            let Some(SubagentEvent::Inspect(request)) = event_rx.recv().await else {
                panic!("expected reconciliation inspection");
            };
            assert_eq!(request.subagent_id, "lost-ack-child");
            drop(request.respond_to);
            let Some(SubagentEvent::ListRunning(request)) = event_rx.recv().await else {
                panic!("expected running inventory");
            };
            drop(request.respond_to);
        });
        let sid = acp::SessionId::new(format!("lost-ack-{}", uuid::Uuid::new_v4()));
        let (handle, _cmd_tx, mut cmd_rx) = make_live_session_handle(&sid, None);
        let parent = sid.0.to_string();
        let session_dir = crate::session::persistence::session_dir(&handle.info);
        let sub_dir = session_dir.join("subagents").join("lost-ack-child");
        write_meta(&sub_dir, &running_meta("lost-ack-child", &parent));
        let before = std::fs::read(sub_dir.join("meta.json")).unwrap();
        agent.insert_resident(&sid, handle);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            agent.list_running_subagents(&parent),
        ).await.expect("lost ACK must not hang");
        assert!(matches!(result, Err(
            xai_grok_tools::implementations::grok_build::task::types::SubagentAccessError::CoordinatorUnavailable
        )));
        assert_eq!(std::fs::read(sub_dir.join("meta.json")).unwrap(), before);
        assert!(cmd_rx.try_recv().is_err());
        receiver.await.unwrap();
        std::fs::remove_dir_all(sub_dir.parent().unwrap()).unwrap();
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn subagent_get_lost_terminal_refresh_ack_is_an_error() {
    let local = tokio::task::LocalSet::new();
    local.run_until(async {
        let agent = build_minimal_agent_for_tests();
        let mut event_rx = agent.subagent_event_rx.borrow_mut().take().unwrap()
            .into_event_receiver();
        let req = acp::ExtRequest::new(
            "x.ai/subagent/get",
            serde_json::value::to_raw_value(&serde_json::json!({
                "subagentId": "completed-child", "block": false
            })).unwrap().into(),
        );
        let responder = async move {
            let Some(SubagentEvent::Query(request)) = event_rx.recv().await else {
                panic!("expected snapshot query");
            };
            assert_eq!(request.subagent_id, "completed-child");
            let mut snapshot = running_inspection("completed-child", "parent").snapshot;
            snapshot.status = SubagentSnapshotStatus::Completed {
                output: "committed output".into(),
                tool_calls: 3,
                turns: 2,
                worktree_path: None,
            };
            request.respond_to.send(Some(snapshot)).unwrap();
            let Some(SubagentEvent::Inspect(request)) = event_rx.recv().await else {
                panic!("expected one terminal refresh");
            };
            assert_eq!(request.subagent_id, "completed-child");
            drop(request.respond_to);
        };
        let (response, ()) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            async { tokio::join!(crate::extensions::task::handle_subagent(&agent, &req), responder) },
        ).await.expect("terminal refresh must not hang");
        let response = response.expect("public error envelope");
        let value: serde_json::Value = serde_json::from_str(response.0.get()).unwrap();
        assert!(value["result"].is_null());
        assert_eq!(value["error"].as_str(), Some(
            "Subagent request not confirmed: coordinator unavailable or response lost"
        ));
    }).await;
}

#[test]
fn release_evicts_the_registry_heal_lock() {
    let registry = super::super::session_registry::SessionRegistry::default();
    let sid = acp::SessionId::new("s-heal-evict");
    let _ = registry.live_orphan_heal_lock(&sid);
    assert_eq!(registry.counts().live_orphan_heal_locks, 1);
    registry.release(&sid);
    assert_eq!(registry.counts().live_orphan_heal_locks, 0);
}

#[test]
fn registry_heal_lock_reuses_the_same_arc_until_release() {
    let registry = super::super::session_registry::SessionRegistry::default();
    let sid = acp::SessionId::new("s-heal-race");
    let inflight = registry.live_orphan_heal_lock(&sid);
    assert_eq!(registry.counts().live_orphan_heal_locks, 1);
    let again = registry.live_orphan_heal_lock(&sid);
    assert!(
        std::sync::Arc::ptr_eq(&inflight, &again),
        "overlapping ticks must share the registry mutex"
    );
    drop(again);
    registry.release(&sid);
    assert_eq!(
        registry.counts().live_orphan_heal_locks,
        0,
        "release must drop the retained lock even while an Arc clone is held"
    );
    drop(inflight);
}

#[tokio::test(flavor = "current_thread")]
async fn subagent_get_query_transport_errors_skip_terminal_refresh() {
    let local = tokio::task::LocalSet::new();
    local.run_until(async {
        for lose_ack in [false, true] {
            let agent = build_minimal_agent_for_tests();
            let mut event_rx = agent.subagent_event_rx.borrow_mut().take().unwrap()
                .into_event_receiver();
            if !lose_ack {
                event_rx.close();
            }
            let req = acp::ExtRequest::new(
                "x.ai/subagent/get",
                serde_json::value::to_raw_value(&serde_json::json!({
                    "subagentId": "child", "block": false
                })).unwrap().into(),
            );
            let responder = async {
                if lose_ack {
                    let Some(SubagentEvent::Query(request)) = event_rx.recv().await else {
                        panic!("expected query");
                    };
                    assert_eq!(request.subagent_id, "child");
                    assert!(!request.block);
                    assert_eq!(request.timeout_ms, Some(30_000));
                    drop(request.respond_to);
                }
            };
            let (response, ()) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                async { tokio::join!(crate::extensions::task::handle_subagent(&agent, &req), responder) },
            ).await.expect("query loss must not hang");
            let response = response.expect("public error envelope");
            let value: serde_json::Value = serde_json::from_str(response.0.get()).unwrap();
            assert!(value["result"].is_null());
            assert_eq!(value["error"].as_str(), Some(
                "Subagent request not confirmed: coordinator unavailable or response lost"
            ));
            assert!(event_rx.try_recv().is_err(), "failed query must not issue inspect");
        }
    }).await;
}
