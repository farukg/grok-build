use super::*;
use crate::views::dashboard::{DashboardRowId, Filter, SessionLane};

fn broadcast(app: &mut AppView, payload: serde_json::Value) {
    let (response_tx, _response_rx) = tokio::sync::oneshot::channel();
    assert!(crate::app::acp_handler::handle(
        xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
            request: acp::ExtNotification::new(
                "x.ai/sessions/changed",
                serde_json::value::to_raw_value(&payload).unwrap().into(),
            ),
            response_tx,
        }),
        app,
    ));
}

fn painted_dashboard(app: &mut AppView) -> String {
    let (mut terminal, _frames) = crate::test_util::test_terminal();
    terminal.resize(ratatui::layout::Rect::new(0, 0, 140, 30)).unwrap();
    app.draw(&mut terminal);
    let buf = terminal.completed_buffer();
    (0..buf.area.height).map(|y| {
        (0..buf.area.width).filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol()))
            .collect::<String>()
    }).collect::<Vec<_>>().join("\n")
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn acp_roster_batch_reconciles_workspace_lanes_saved_layout_and_prompt_identity() {
    use crate::app::agent_view::{AgentRole, PromptTarget};
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    std::fs::write(&config, "[dashboard]\npinned = ['top:broadcast-child', 'top:generic-child']\nreorder = ['top:generic-child', 'top:broadcast-child']\n").unwrap();
    let mut app = test_app_with_agent();
    app.leader_mode = true;
    app.workspace_dashboard_enabled = true;
    app.dashboard_persisted = crate::views::dashboard::state::load_persisted_from_path(&config);
    app.workspace_membership.set_snapshot_for_test(workspace_snapshot(vec![
        member("saved-root", "Visible root"),
        member("broadcast-child", "Broadcast child"),
        member("generic-child", "Generic child"),
    ]));
    let _ = dispatch(Action::LoadSession("generic-child".into(), None, false), &mut app);
    let generic = app.agents.find_by_session_id("generic-child").unwrap();
    let _ = dispatch(Action::TaskComplete(TaskResult::SessionLoaded {
        agent_id: generic,
        session_id: acp::SessionId::new("generic-child"),
        models: None,
        modes: None,
        code_restored: false,
        restore_summary: None,
        restore_degree: None,
        running_prompt_id: None,
    }), &mut app);
    app.agents.get_mut(&generic).unwrap().display_name = Some("Generic child".into());
    assert!(matches!(app.agents.get(&generic).unwrap().role, AgentRole::Root));
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    broadcast(&mut app, serde_json::json!({
        "upserted": [
            {"sessionId": "broadcast-child", "title": "Broadcast child", "cwd": "/repo",
             "sessionKind": "subagent", "parentSessionId": "absent-parent", "activity": "idle"},
            {"sessionId": "generic-child", "title": "Generic child", "cwd": "/repo",
             "sessionKind": "subagent", "parentSessionId": "absent-parent", "activity": "idle"}
        ]
    }));
    let dashboard = app.dashboard.as_ref().unwrap();
    assert_eq!(dashboard.child_rows.len(), 2);
    assert!(dashboard.pinned.contains(&DashboardRowId::Roster { session_id: "broadcast-child".into() }));
    assert!(dashboard.pinned.contains(&DashboardRowId::TopLevel(generic)));
    assert_eq!(dashboard.reorder, vec![
        DashboardRowId::TopLevel(generic),
        DashboardRowId::Roster { session_id: "broadcast-child".into() },
    ]);
    let main = workspace_rows(&app, &Filter::None).0;
    assert_eq!(main.iter().map(|row| row.session_id.as_deref()).collect::<Vec<_>>(), vec![Some("saved-root")]);
    let main = painted_dashboard(&mut app);
    assert!(main.contains("Visible root"));
    assert!(!main.contains("Broadcast child"));
    assert!(!main.contains("Generic child"));
    app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::Subagents);
    let children = workspace_rows(&app, &Filter::None).0;
    assert_eq!(children.iter().map(|row| row.session_id.as_deref()).collect::<Vec<_>>(),
        vec![Some("generic-child"), Some("broadcast-child")]);
    let children = painted_dashboard(&mut app);
    assert!(children.contains("Broadcast child"));
    assert!(children.contains("Generic child"));
    assert!(!children.contains("Visible root"));
    let view = app.agents.get_mut(&generic).unwrap();
    assert_eq!(view.role.prompt_target(), PromptTarget::ChildOf(acp::SessionId::new("absent-parent")));
    view.session.enqueue_bash_command("pwd".into());
    let effects = maybe_drain_queue(view, &mut Vec::new()).effects;
    assert!(matches!(effects.as_slice(),
        [Effect::SendBashCommand { session_id, target: PromptTarget::ChildOf(parent), .. }]
        if session_id.0.as_ref() == "generic-child" && parent.0.as_ref() == "absent-parent"));
    broadcast(&mut app, serde_json::json!({"removed": ["broadcast-child"]}));
    assert!(!app.dashboard.as_ref().unwrap().child_rows.contains_key("broadcast-child"));
    let children = workspace_rows(&app, &Filter::None).0;
    assert_eq!(children.iter().map(|row| row.session_id.as_deref()).collect::<Vec<_>>(), vec![Some("generic-child")]);
    assert!(!painted_dashboard(&mut app).contains("Broadcast child"));
    app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::Main);
    let main = workspace_rows(&app, &Filter::None).0;
    assert!(main.iter().any(|row| row.session_id.as_deref() == Some("broadcast-child")));
    assert!(!main.iter().any(|row| row.session_id.as_deref() == Some("generic-child")));
    let main = painted_dashboard(&mut app);
    assert!(main.contains("Broadcast child"));
    assert!(!main.contains("Generic child"));
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn roster_reconciliation_prepares_membership_reused_by_frames_and_dispatch() {
    for leader_mode in [true, false] {
        let mut app = test_app_with_agent();
        app.workspace_dashboard_enabled = true;
        app.leader_mode = leader_mode;
        app.workspace_membership.set_snapshot_for_test(workspace_snapshot(vec![
            member("saved-root", "Visible root"),
            member("saved-child", "Hidden child"),
        ]));
        app.active_view = ActiveView::AgentDashboard;
        ensure_dashboard_state(&mut app);
        let mut child = idle_roster_entry("saved-child", "Hidden child");
        child.session_kind = Some(crate::app::roster::SESSION_KIND_SUBAGENT.into());
        child.parent_session_id = Some("detached-parent".into());
        let result = if leader_mode {
            TaskResult::RosterLoaded { sessions: vec![child] }
        } else {
            TaskResult::DashboardSessionsLoaded { sessions: vec![child] }
        };
        let _ = dispatch(Action::TaskComplete(result), &mut app);
        let prepared = &app.dashboard.as_ref().unwrap().child_rows;
        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared.get("saved-child"), Some(&DashboardRowId::Roster {
            session_id: "saved-child".into(),
        }));
        let allocation = prepared.keys().next().unwrap().as_ptr();
        let (mut terminal, _frames) = crate::test_util::test_terminal();
        terminal.resize(ratatui::layout::Rect::new(0, 0, 140, 30)).unwrap();
        for _ in 0..3 {
            app.draw(&mut terminal);
            let rows = workspace_rows(&app, &Filter::None).0;
            assert!(rows.iter().any(|row| row.session_id.as_deref() == Some("saved-root")));
            assert!(!rows.iter().any(|row| row.session_id.as_deref() == Some("saved-child")));
            let buf = terminal.completed_buffer();
            let painted = (0..buf.area.height).flat_map(|y| {
                (0..buf.area.width).filter_map(move |x| buf.cell((x, y)).map(|cell| cell.symbol()))
            }).collect::<String>();
            assert!(painted.contains("Visible root"));
            assert!(!painted.contains("Hidden child"));
            assert_eq!(app.dashboard.as_ref().unwrap().child_rows.keys().next().unwrap().as_ptr(), allocation);
        }
        let result = if leader_mode {
            TaskResult::RosterLoaded { sessions: vec![] }
        } else {
            TaskResult::DashboardSessionsLoaded { sessions: vec![] }
        };
        let _ = dispatch(Action::TaskComplete(result), &mut app);
        assert!(app.dashboard.as_ref().unwrap().child_rows.is_empty());
        assert!(workspace_rows(&app, &Filter::None).0.iter()
            .any(|row| row.session_id.as_deref() == Some("saved-child")));
    }
}
