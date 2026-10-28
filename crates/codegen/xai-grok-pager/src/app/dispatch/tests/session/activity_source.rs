use super::*;
use crate::app::roster::SessionActivitySnapshot;

fn loaded(app: &mut AppView, id: AgentId, sid: &str) {
    dispatch(
        Action::TaskComplete(TaskResult::SessionLoaded {
            agent_id: id,
            session_id: acp::SessionId::new(sid),
            models: None,
            modes: None,
            code_restored: false,
            restore_summary: None,
            restore_degree: None,
            running_prompt_id: None,
        }),
        app,
    );
}

fn hydrate(app: &mut AppView, id: AgentId, sid: &str, millis: i64) {
    dispatch(
        Action::TaskComplete(TaskResult::SessionMetaFromDisk {
            agent_id: id,
            activity: Some(SessionActivitySnapshot {
                session_id: acp::SessionId::new(sid),
                last_change_unix_ms: millis,
            }),
            title: None,
            last_turn_summary: None,
            last_turn_summary_gen: 0,
        }),
        app,
    );
}

fn output(app: &mut AppView, sid: &str, replay: bool) {
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let meta = serde_json::json!({"isReplay": replay})
        .as_object()
        .unwrap()
        .clone();
    crate::app::acp_handler::handle(
        xai_acp_lib::AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs {
            request: acp::SessionNotification::new(
                acp::SessionId::new(sid),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    acp::ContentBlock::Text(acp::TextContent::new("actual stream output")),
                )),
            )
            .meta(meta),
            response_tx: tx,
        }),
        app,
    );
}

#[test]
fn viewing_and_reloading_preserve_persisted_activity_per_session() {
    let mut app = test_app();
    dispatch(Action::LoadSession("older".into(), None, false), &mut app);
    let id = AgentId(0);
    assert_eq!(expect_agent(&app, id).last_active_at, None);
    loaded(&mut app, id, "older");
    assert_eq!(expect_agent(&app, id).last_active_at, None);
    let old_ms = chrono::Utc::now().timestamp_millis() - 86_400_000;
    hydrate(&mut app, id, "older", old_ms);
    let old = expect_agent(&app, id).last_active_at.unwrap();
    assert!(old.elapsed() >= std::time::Duration::from_secs(86_399));
    loaded(&mut app, id, "older");
    hydrate(&mut app, id, "older", old_ms);
    assert_eq!(expect_agent(&app, id).last_active_at, Some(old));
    app.agents.get_mut(&id).unwrap().begin_session_reload(7);
    app.agents
        .get_mut(&id)
        .unwrap()
        .finish_session_reload(7, true);
    assert_eq!(expect_agent(&app, id).last_active_at, Some(old));
    loaded(&mut app, id, "different-session");
    hydrate(&mut app, id, "older", old_ms);
    assert_eq!(
        expect_agent(&app, id).last_active_at,
        None,
        "stale metadata belongs to another session"
    );
    hydrate(&mut app, id, "different-session", 0);
    assert_eq!(expect_agent(&app, id).last_active_at, None);
    hydrate(&mut app, id, "different-session", -1);
    assert_eq!(expect_agent(&app, id).last_active_at, None);
}

#[test]
fn known_roster_activity_survives_loading_without_disk_metadata() {
    for leader in [false, true] {
        let mut app = test_app();
        app.leader_mode = leader;
        let mut row = crate::app::agent_test_fixtures::roster_entry(
            "roster-history",
            crate::app::roster::RosterActivity::Idle,
        );
        row.last_change_unix_ms = chrono::Utc::now().timestamp_millis() - 86_400_000;
        let result = if leader {
            TaskResult::RosterLoaded {
                sessions: vec![row],
            }
        } else {
            TaskResult::DashboardSessionsLoaded {
                sessions: vec![row],
            }
        };
        dispatch(Action::TaskComplete(result), &mut app);
        dispatch(
            Action::LoadSession("roster-history".into(), None, false),
            &mut app,
        );
        loaded(&mut app, AgentId(0), "roster-history");
        let activity = expect_agent(&app, AgentId(0)).last_active_at.unwrap();
        assert!(activity.elapsed() >= std::time::Duration::from_secs(86_399));
        loaded(&mut app, AgentId(0), "roster-history");
        assert_eq!(
            expect_agent(&app, AgentId(0)).last_active_at,
            Some(activity)
        );
    }
}

#[test]
fn parent_and_child_live_output_advance_activity_but_replay_does_not() {
    let mut app = test_app();
    dispatch(
        Action::LoadSession("parent-activity".into(), None, false),
        &mut app,
    );
    let parent = AgentId(0);
    loaded(&mut app, parent, "parent-activity");
    let mut child_view = crate::app::agent_view::test_fixtures::make_agent();
    child_view.session.session_id = Some(acp::SessionId::new("child-activity"));
    let child = AgentId(1);
    crate::app::session_views::test_support::link_child(
        &mut app.agents,
        parent,
        child,
        child_view,
        Instant::now(),
    );
    let old_ms = chrono::Utc::now().timestamp_millis() - 86_400_000;
    for (id, sid) in [(parent, "parent-activity"), (child, "child-activity")] {
        hydrate(&mut app, id, sid, old_ms);
        let old = expect_agent(&app, id).last_active_at.unwrap();
        output(&mut app, sid, true);
        assert_eq!(expect_agent(&app, id).last_active_at, Some(old));
        let before = Instant::now();
        output(&mut app, sid, false);
        let live = expect_agent(&app, id).last_active_at.unwrap();
        assert!(live >= before);
        hydrate(&mut app, id, sid, old_ms);
        assert_eq!(
            expect_agent(&app, id).last_active_at,
            Some(live),
            "late disk reads cannot erase live activity"
        );
    }
}

fn acp_observation(app: &mut AppView, sid: &str, update: acp::SessionUpdate, replay: bool) {
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    crate::app::acp_handler::handle(
        xai_acp_lib::AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs {
            request: acp::SessionNotification::new(acp::SessionId::new(sid), update).meta(Some(
                serde_json::json!({"isReplay": replay})
                    .as_object()
                    .unwrap()
                    .clone(),
            )),
            response_tx,
        }),
        app,
    );
}

#[test]
fn root_and_child_observational_updates_do_not_advance_activity() {
    let mut app = test_app();
    dispatch(
        Action::LoadSession("observe-parent".into(), None, false),
        &mut app,
    );
    loaded(&mut app, AgentId(0), "observe-parent");
    let mut child_view = crate::app::agent_view::test_fixtures::make_agent();
    child_view.session.session_id = Some(acp::SessionId::new("observe-child"));
    crate::app::session_views::test_support::link_child(
        &mut app.agents,
        AgentId(0),
        AgentId(1),
        child_view,
        Instant::now(),
    );
    for (id, sid) in [
        (AgentId(0), "observe-parent"),
        (AgentId(1), "observe-child"),
    ] {
        hydrate(&mut app, id, sid, 1_234);
        let original = expect_agent(&app, id).last_active_at;
        for loading in [true, false] {
            app.agents.get_mut(&id).unwrap().session.loading_replay = loading;
            for replay in [true, false] {
                for update in [
                    acp::SessionUpdate::UsageUpdate(acp::UsageUpdate::new(42_000, 100_000)),
                    acp::SessionUpdate::AvailableCommandsUpdate(acp::AvailableCommandsUpdate::new(
                        Vec::new(),
                    )),
                    acp::SessionUpdate::Plan(acp::Plan::new(Vec::new())),
                ] {
                    acp_observation(&mut app, sid, update, replay);
                    assert_eq!(expect_agent(&app, id).last_active_at, original);
                }
            }
        }
    }
}

#[test]
fn a_later_persisted_snapshot_can_replace_an_earlier_roster_snapshot() {
    let mut app = test_app();
    dispatch(
        Action::LoadSession("snapshot-progress".into(), None, false),
        &mut app,
    );
    loaded(&mut app, AgentId(0), "snapshot-progress");
    hydrate(&mut app, AgentId(0), "snapshot-progress", 1_234);
    let earlier = expect_agent(&app, AgentId(0)).last_active_at.unwrap();
    hydrate(&mut app, AgentId(0), "snapshot-progress", 9_876);
    let later = expect_agent(&app, AgentId(0)).last_active_at.unwrap();
    assert_eq!(
        later.duration_since(earlier),
        std::time::Duration::from_millis(8_642)
    );
    hydrate(&mut app, AgentId(0), "snapshot-progress", 9_876);
    assert_eq!(expect_agent(&app, AgentId(0)).last_active_at, Some(later));
    hydrate(&mut app, AgentId(0), "snapshot-progress", 1_234);
    assert_eq!(expect_agent(&app, AgentId(0)).last_active_at, Some(later));
}

#[test]
fn picker_restores_activity_before_disk_hydration_without_inventing_absent_activity() {
    for last_active_at in [None, chrono::DateTime::from_timestamp_millis(1_234)] {
        let mut app = test_app_with_agent();
        let mut entry = make_conversation_entry("picker-activity");
        entry.updated_at = chrono::DateTime::from_timestamp_millis(9_876).unwrap();
        entry.last_active_at = last_active_at;
        open_session_picker_with(&mut app, vec![entry]);
        dispatch(Action::PickSession(0), &mut app);
        let id = app.agents.find_by_session_id("picker-activity").unwrap();
        let expected = SessionActivitySnapshot {
            session_id: acp::SessionId::new("picker-activity"),
            last_change_unix_ms: if last_active_at.is_some() {
                1_234
            } else {
                9_876
            },
        }
        .instant();
        assert_eq!(expect_agent(&app, id).last_active_at, expected);
        loaded(&mut app, id, "picker-activity");
        assert_eq!(expect_agent(&app, id).last_active_at, expected);
    }
}

fn xai_update(
    app: &mut AppView,
    sid: &str,
    update: xai_grok_shell::extensions::notification::SessionUpdate,
    meta: serde_json::Value,
) {
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    let notification = xai_grok_shell::extensions::notification::SessionNotification {
        session_id: acp::SessionId::new(sid),
        update,
        meta: Some(meta),
    };
    crate::app::acp_handler::handle(
        xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
            request: xai_grok_test_support::acp_fixtures::ext_notification(
                "x.ai/session_notification",
                &notification,
            ),
            response_tx,
        }),
        app,
    );
}

#[test]
fn background_and_title_snapshots_do_not_advance_parent_or_child_activity() {
    use xai_grok_shell::extensions::notification::SessionUpdate as XaiUpdate;
    let mut app = test_app();
    dispatch(
        Action::LoadSession("snapshot-parent".into(), None, false),
        &mut app,
    );
    loaded(&mut app, AgentId(0), "snapshot-parent");
    let mut child_view = crate::app::agent_view::test_fixtures::make_agent();
    child_view.session.session_id = Some(acp::SessionId::new("snapshot-child"));
    crate::app::session_views::test_support::link_child(
        &mut app.agents,
        AgentId(0),
        AgentId(1),
        child_view,
        Instant::now(),
    );
    for (id, sid) in [
        (AgentId(0), "snapshot-parent"),
        (AgentId(1), "snapshot-child"),
    ] {
        hydrate(&mut app, id, sid, 1_234);
        let original = expect_agent(&app, id).last_active_at;
        for loading in [true, false] {
            app.agents.get_mut(&id).unwrap().session.loading_replay = loading;
            for replay in [true, false] {
                for update in [
                    XaiUpdate::BackgroundTasks {
                        tasks: Vec::new(),
                        truncated: false,
                    },
                    XaiUpdate::SessionSummaryGenerated {
                        session_summary: "observed title".into(),
                    },
                ] {
                    xai_update(
                        &mut app,
                        sid,
                        update,
                        serde_json::json!({"isReplay": replay}),
                    );
                    assert_eq!(expect_agent(&app, id).last_active_at, original);
                }
            }
        }
    }
}

#[test]
fn every_admitted_child_argument_delta_advances_activity_but_replay_stale_and_duplicates_do_not() {
    use xai_grok_shell::extensions::notification::SessionUpdate as XaiUpdate;
    let mut app = test_app();
    dispatch(
        Action::LoadSession("delta-parent".into(), None, false),
        &mut app,
    );
    loaded(&mut app, AgentId(0), "delta-parent");
    let parent_activity = expect_agent(&app, AgentId(0)).last_active_at;
    app.agents
        .get_mut(&AgentId(0))
        .unwrap()
        .subagent_sessions
        .insert(
            "delta-child".into(),
            make_test_subagent("delta-child", "delta-child"),
        );
    let mut child_view = crate::app::agent_view::test_fixtures::make_agent();
    child_view.session.session_id = Some(acp::SessionId::new("delta-child"));
    child_view.session.current_prompt_id = Some("current".into());
    child_view.session.state = AgentState::TurnRunning;
    crate::app::session_views::test_support::link_child(
        &mut app.agents,
        AgentId(0),
        AgentId(1),
        child_view,
        Instant::now(),
    );
    hydrate(&mut app, AgentId(1), "delta-child", 1_234);
    let delta = || XaiUpdate::ToolCallDeltaChunk {
        tool_call_id: Some("call-1".into()),
        tool_index: 0,
        name: Some("read_file".into()),
        arguments_delta: Some("fragment".into()),
    };
    let original = expect_agent(&app, AgentId(1)).last_active_at;
    for meta in [
        serde_json::json!({"promptId": "current", "isReplay": true, "eventId": "delta-child-1"}),
        serde_json::json!({"promptId": "old", "eventId": "delta-child-1"}),
    ] {
        xai_update(&mut app, "delta-child", delta(), meta);
        assert_eq!(expect_agent(&app, AgentId(1)).last_active_at, original);
    }
    for seq in [1, 2] {
        let before = Instant::now();
        let meta =
            serde_json::json!({"promptId": "current", "eventId": format!("delta-child-{seq}")});
        xai_update(&mut app, "delta-child", delta(), meta.clone());
        let accepted = expect_agent(&app, AgentId(1)).last_active_at.unwrap();
        assert!(
            accepted >= before,
            "unchanged tracker labels do not mean no new bytes"
        );
        xai_update(&mut app, "delta-child", delta(), meta);
        assert_eq!(
            expect_agent(&app, AgentId(1)).last_active_at,
            Some(accepted)
        );
    }
    assert_eq!(
        expect_agent(&app, AgentId(0)).last_active_at,
        parent_activity
    );
}

#[test]
fn historical_child_compaction_cleanup_does_not_become_live_activity() {
    use xai_grok_shell::extensions::notification::SessionUpdate as XaiUpdate;
    let mut app = test_app();
    dispatch(
        Action::LoadSession("compact-parent".into(), None, false),
        &mut app,
    );
    loaded(&mut app, AgentId(0), "compact-parent");
    let mut child_view = crate::app::agent_view::test_fixtures::make_agent();
    child_view.session.session_id = Some(acp::SessionId::new("compact-child"));
    crate::app::session_views::test_support::link_child(
        &mut app.agents,
        AgentId(0),
        AgentId(1),
        child_view,
        Instant::now(),
    );
    hydrate(&mut app, AgentId(1), "compact-child", 1_234);
    let original = expect_agent(&app, AgentId(1)).last_active_at;
    for loading in [true, false] {
        let child = app.agents.get_mut(&AgentId(1)).unwrap();
        child.session.loading_replay = loading;
        child
            .session
            .start_command(crate::app::agent::AgentCommand::SwitchModelCompact);
        xai_update(
            &mut app,
            "compact-child",
            XaiUpdate::AutoCompactCompleted {
                tokens_before: Some(90_000),
                tokens_after: 20_000,
                elapsed_ms: Some(100),
                summary_preview: None,
            },
            serde_json::json!({"isReplay": true}),
        );
        assert_eq!(expect_agent(&app, AgentId(1)).last_active_at, original);
    }
}
