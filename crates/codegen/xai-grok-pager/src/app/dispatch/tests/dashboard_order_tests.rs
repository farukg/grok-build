use super::*;
use crate::app::agent::AgentId;
use crate::app::agent_view::{AgentRole, ChildLink};
use crate::app::roster::{RosterActivity, RosterEntry};
use crate::views::dashboard::{DashboardRowId, Filter, Grouping, PersistedRowId, SessionLane};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
enum Source {
    Local,
    Leader,
    Workspace,
}

const NAMES: [&str; 11] = [
    "QueuedRoot", "RemoteRoot", "WorkingChild", "HistRootA", "HistChildB",
    "HistChildC", "HistRootD", "TieChildA", "TieRootZ", "UnknownChildA", "UnknownRootZ",
];

fn metadata_row(id: &str, parent: Option<&str>, active: Option<i64>, updated: i64) -> RosterEntry {
    let timestamp = |millis| chrono::DateTime::from_timestamp_millis(millis).unwrap().to_rfc3339();
    let summary: xai_grok_shell::session::persistence::Summary = serde_json::from_value(
        serde_json::json!({
            "info": {"id": id, "cwd": format!("/history/{id}")},
            "session_summary": id,
            "generated_title": id,
            "created_at": "2020-01-01T00:00:00Z",
            "updated_at": timestamp(updated),
            "last_active_at": active.map(timestamp),
            "num_messages": 12,
            "current_model_id": "grok-test",
            "session_kind": parent.map(|_| "subagent"),
            "parent_session_id": parent,
        }),
    ).unwrap();
    RosterEntry {
        session_id: id.into(),
        title: summary.display_title_opt(),
        cwd: summary.info.cwd.clone(),
        is_worktree: false,
        session_kind: summary.session_kind.clone(),
        parent_session_id: summary.parent_session_id.clone(),
        model_id: None,
        yolo: false,
        activity: RosterActivity::Dormant,
        last_turn_summary: None,
        resident: false,
        last_change_unix_ms: summary.last_change_unix_ms(),
        origin: Default::default(),
    }
}

fn mixed_app(source: Source) -> AppView {
    let mut app = test_app();
    app.leader_mode = matches!(source, Source::Leader);
    app.workspace_dashboard_enabled = matches!(source, Source::Workspace);
    let now = Instant::now();
    let wall = chrono::Utc::now().timestamp_millis();
    let mut queued = crate::app::agent_view::test_fixtures::make_agent();
    queued.display_name = Some("QueuedRoot".into());
    queued.session.session_id = Some(acp::SessionId::new("QueuedRoot"));
    queued.session.cwd = "/z/queued".into();
    queued.last_active_at = Some(now - Duration::from_secs(80));
    queued.turn_started_at = Some(now - Duration::from_secs(200));
    queued.session.enqueue_prompt("queued work".into());
    app.agents.insert(AgentId(90), queued);
    let mut child = crate::app::agent_view::test_fixtures::make_agent();
    child.display_name = Some("WorkingChild".into());
    child.session.session_id = Some(acp::SessionId::new("WorkingChild"));
    child.session.cwd = "/a/child".into();
    child.last_active_at = Some(now - Duration::from_secs(100));
    child.turn_started_at = Some(now - Duration::from_secs(150));
    child.session.bg_tasks.insert("task".into(),
        crate::app::agent_test_fixtures::running_bg_task("task", false));
    child.role = AgentRole::Child(ChildLink {
        parent: Some(AgentId(90)), parent_session_id: acp::SessionId::new("QueuedRoot"),
        subagent_id: "worker".into(), started_at: now - Duration::from_secs(150),
    });
    app.agents.insert(AgentId(1), child);
    let mut roster = vec![
        metadata_row("RemoteRoot", None, Some(wall - 90_000), wall),
        metadata_row("HistRootA", None, Some(wall - 200_000), wall + 50_000),
        metadata_row("HistChildB", Some("unloaded-historical-parent"), None, wall - 210_000),
        metadata_row("HistChildC", Some("unloaded-historical-parent"), Some(wall - 220_000), wall + 90_000),
        metadata_row("HistRootD", None, Some(wall - 230_000), wall),
        metadata_row("TieChildA", Some("unloaded-historical-parent"), Some(wall - 240_000), wall),
        metadata_row("TieRootZ", None, Some(wall - 240_000), wall),
        metadata_row("UnknownChildA", Some("unloaded-historical-parent"), None, wall),
        metadata_row("UnknownRootZ", None, None, wall),
    ];
    roster[0].activity = RosterActivity::Working;
    roster[0].resident = true;
    roster[3].activity = RosterActivity::Completed;
    roster[4].activity = RosterActivity::Dead;
    roster[7].last_change_unix_ms = 0;
    roster[8].last_change_unix_ms = -1;
    roster.reverse();
    if matches!(source, Source::Workspace) {
        let mut members = roster.iter().filter(|row| row.parent_session_id.is_none()).map(|row| {
            let mut member = crate::app::workspace_test_fixtures::member(&row.session_id, &row.session_id);
            member.cwd = Some(row.cwd.clone());
            member.last_change_unix_ms = wall + 999_000;
            if row.session_id == "HistRootD" {
                member.pin_rank = Some(1);
                member.order_rank = Some(1);
            }
            member
        }).collect::<Vec<_>>();
        members.push(crate::app::workspace_test_fixtures::member("QueuedRoot", "QueuedRoot"));
        app.workspace_membership.set_snapshot_for_test(
            crate::app::workspace_test_fixtures::snapshot(members));
    }
    let mut saved = crate::views::dashboard::PersistedDashboard::defaults();
    saved.pinned.insert(PersistedRowId::TopLevel { session_id: "HistRootD".into() });
    saved.pinned.insert(PersistedRowId::TopLevel { session_id: "UnknownChildA".into() });
    saved.reorder = ["UnknownRootZ", "UnknownChildA", "HistRootD"].map(|id|
        PersistedRowId::TopLevel { session_id: id.into() }).into();
    app.dashboard_persisted = Some(saved);
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    let result = match source {
        Source::Leader => TaskResult::RosterLoaded { sessions: roster },
        Source::Local | Source::Workspace => TaskResult::DashboardSessionsLoaded { sessions: roster },
    };
    dispatch(Action::TaskComplete(result), &mut app);
    let state = app.dashboard.as_mut().unwrap();
    state.collapsed_sections.clear();
    state.preview_enabled = false;
    state.idle_show_all = true;
    app
}

fn paint_mixed(app: &mut AppView, height: u16) -> String {
    use ratatui::{buffer::Buffer, layout::Rect};
    let source = crate::app::workspace_sync::WorkspaceRowSource::capture(
        &app.agents, &app.workspace_membership, app.home_session_agent,
        app.workspace_dashboard_enabled,
    );
    let roster = app.dashboard_roster().to_vec();
    let area = Rect::new(0, 0, 160, height);
    let mut buffer = Buffer::empty(area);
    crate::views::dashboard::render_dashboard(
        &mut buffer, area, app.dashboard.as_mut().unwrap(), &mut app.agents,
        &app.registry, None, &roster, app.workspace_dashboard_enabled, source.inputs(),
        None, false, None, None,
    );
    (0..height).map(|y| (0..area.width).map(|x|
        buffer.cell((x, y)).unwrap().symbol()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

fn painted_names(painted: &str) -> Vec<&str> {
    painted.lines().flat_map(|line| NAMES.iter().copied().filter(move |name| line.contains(name))).collect()
}

#[test]
fn mixed_metadata_runtime_and_saved_layout_have_one_order_in_every_lane() {
    for source in [Source::Local, Source::Leader, Source::Workspace] {
        let mut app = mixed_app(source);
        for (lane, expected) in [
            (SessionLane::Main, vec!["QueuedRoot", "RemoteRoot", "HistRootA", "HistRootD", "TieRootZ", "UnknownRootZ"]),
            (SessionLane::Subagents, vec!["WorkingChild", "HistChildB", "HistChildC", "TieChildA", "UnknownChildA"]),
            (SessionLane::All, NAMES.to_vec()),
        ] {
            for grouping in [Grouping::State, Grouping::Directory] {
                if matches!(source, Source::Workspace) {
                    let mut snapshot = app.workspace_membership.snapshot().unwrap().clone();
                    snapshot.grouping = match grouping {
                        Grouping::State => xai_grok_dashboard_store::Grouping::State,
                        Grouping::Directory => xai_grok_dashboard_store::Grouping::Directory,
                    };
                    app.workspace_membership.set_snapshot_for_test(snapshot);
                }
                let state = app.dashboard.as_mut().unwrap();
                state.set_session_lane(lane);
                state.grouping = grouping;
                state.collapsed_sections.clear();
                state.focus_row(DashboardRowId::TopLevel(AgentId(1)));
                let before = state.reorder.clone();
                let painted = paint_mixed(&mut app, 100);
                assert_eq!(painted_names(&painted), expected, "{source:?} {lane:?} {grouping:?}");
                assert_eq!(app.dashboard.as_ref().unwrap().reorder, before);
                assert!(app.dashboard.as_ref().unwrap().section_rects.is_empty(),
                    "interleaved states must not produce duplicate section cursor identities");
                if lane != SessionLane::Main {
                    assert_eq!(app.dashboard.as_ref().unwrap().selected,
                        Some(DashboardRowId::TopLevel(AgentId(1))));
                }
                assert_eq!(painted_names(&paint_mixed(&mut app, 100)), expected);
                press_page_key(&mut app, crossterm::event::KeyCode::End);
                let expected_last = match lane {
                    SessionLane::Main | SessionLane::All => "UnknownRootZ",
                    SessionLane::Subagents => "UnknownChildA",
                };
                let selected = app.dashboard.as_ref().unwrap().selected.as_ref().unwrap();
                match selected {
                    DashboardRowId::Roster { session_id } | DashboardRowId::Workspace { session_id } =>
                        assert_eq!(session_id, expected_last),
                    DashboardRowId::TopLevel(id) => panic!("unexpected final live row: {id:?}"),
                }
            }
        }
    }
}

#[test]
fn refresh_reorders_by_activity_without_retargeting_selected_row_or_actions() {
    for source in [Source::Local, Source::Leader, Source::Workspace] {
        let mut app = mixed_app(source);
        app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::All);
        let selected = DashboardRowId::TopLevel(AgentId(1));
        app.dashboard.as_mut().unwrap().focus_row(selected.clone());
        paint_mixed(&mut app, 100);
        let mut roster = app.dashboard_roster().to_vec();
        let row = roster.iter_mut().find(|row| row.session_id == "HistChildC").unwrap();
        row.activity = RosterActivity::Working;
        row.last_change_unix_ms = chrono::Utc::now().timestamp_millis();
        let result = match source {
            Source::Leader => TaskResult::RosterLoaded { sessions: roster },
            Source::Local | Source::Workspace => TaskResult::DashboardSessionsLoaded { sessions: roster },
        };
        dispatch(Action::TaskComplete(result), &mut app);
        assert_eq!(painted_names(&paint_mixed(&mut app, 100)), vec![
            "HistChildC", "QueuedRoot", "RemoteRoot", "WorkingChild", "HistRootA", "HistChildB",
            "HistRootD", "TieChildA", "TieRootZ", "UnknownChildA", "UnknownRootZ",
        ]);
        assert_eq!(app.dashboard.as_ref().unwrap().selected, Some(selected.clone()));
        dispatch(Action::DashboardBeginRename, &mut app);
        assert_eq!(app.dashboard.as_ref().unwrap().rename.as_ref().unwrap().row, selected);
        app.dashboard.as_mut().unwrap().rename = None;
        let order_before = app.dashboard.as_ref().unwrap().reorder.clone();
        assert!(dispatch(Action::DashboardReorderUp, &mut app).is_empty());
        assert_eq!(app.dashboard.as_ref().unwrap().reorder, order_before);
        assert!(app.dashboard.as_ref().unwrap().error_toast.as_ref().unwrap().contains("last activity"));
        let state = app.dashboard.as_mut().unwrap();
        state.filter = Filter::Substring("HistChild".into());
        let filtered = paint_mixed(&mut app, 60);
        assert_eq!(painted_names(&filtered), vec!["HistChildC", "HistChildB"]);
        assert_eq!(app.dashboard.as_ref().unwrap().selected, None);
        dispatch(Action::DashboardBeginRename, &mut app);
        assert!(app.dashboard.as_ref().unwrap().rename.is_none());
    }
}

#[test]
fn paging_uses_activity_order_across_root_child_and_unknown_boundaries() {
    for source in [Source::Local, Source::Leader, Source::Workspace] {
        let mut app = mixed_app(source);
        if matches!(source, Source::Workspace) {
            let mut snapshot = app.workspace_membership.snapshot().unwrap().clone();
            snapshot.grouping = xai_grok_dashboard_store::Grouping::Directory;
            app.workspace_membership.set_snapshot_for_test(snapshot);
        }
        let state = app.dashboard.as_mut().unwrap();
        state.set_session_lane(SessionLane::All);
        state.grouping = Grouping::Directory;
        state.focus_new_agent_button();
        paint_mixed(&mut app, 30);
        press_page_key(&mut app, crossterm::event::KeyCode::Home);
        assert_eq!(app.dashboard.as_ref().unwrap().selected,
            Some(DashboardRowId::TopLevel(AgentId(90))));
        assert_eq!(painted_names(&paint_mixed(&mut app, 30)), NAMES[..6]);
        assert_eq!(app.dashboard.as_ref().unwrap().last_viewport_h, 18);
        press_page_key(&mut app, crossterm::event::KeyCode::PageDown);
        let expected = match source {
            Source::Local | Source::Leader => DashboardRowId::Roster { session_id: "HistRootD".into() },
            Source::Workspace => DashboardRowId::Workspace { session_id: "HistRootD".into() },
        };
        assert_eq!(app.dashboard.as_ref().unwrap().selected, Some(expected));
        let page = paint_mixed(&mut app, 30);
        assert!(page.contains("HistRootD"));
        assert!(!page.contains("QueuedRoot"));
        press_page_key(&mut app, crossterm::event::KeyCode::PageUp);
        assert_eq!(app.dashboard.as_ref().unwrap().selected,
            Some(DashboardRowId::TopLevel(AgentId(90))));
        assert!(paint_mixed(&mut app, 30).contains("QueuedRoot"));
        press_page_key(&mut app, crossterm::event::KeyCode::End);
        assert!(paint_mixed(&mut app, 30).contains("UnknownRootZ"));
    }
}

#[test]
fn equal_live_activity_and_unknown_time_use_stable_session_identity() {
    let anchor = Instant::now() - Duration::from_secs(100);
    for ids in [[90, 1], [1, 90]] {
        let mut app = test_app();
        app.dashboard_persisted = Some(crate::views::dashboard::PersistedDashboard::defaults());
        for (id, name) in ids.into_iter().zip(["TieLiveA", "TieLiveZ"]) {
            let mut agent = crate::app::agent_view::test_fixtures::make_agent();
            agent.display_name = Some(name.into());
            agent.session.session_id = Some(acp::SessionId::new(name));
            agent.last_active_at = Some(anchor);
            app.agents.insert(AgentId(id), agent);
        }
        let mut unknown = crate::app::agent_view::test_fixtures::make_agent();
        unknown.display_name = Some("UnknownLive".into());
        unknown.session.session_id = Some(acp::SessionId::new("AUnknown"));
        unknown.last_active_at = None;
        app.agents.insert(AgentId(2), unknown);
        app.active_view = ActiveView::AgentDashboard;
        ensure_dashboard_state(&mut app);
        app.dashboard.as_mut().unwrap().preview_enabled = false;
        for _ in 0..3 {
            let painted = paint_dashboard(&mut app, 40);
            assert!(painted.find("TieLiveA").unwrap() < painted.find("TieLiveZ").unwrap());
            assert!(painted.find("TieLiveZ").unwrap() < painted.find("UnknownLive").unwrap());
        }
    }
}
