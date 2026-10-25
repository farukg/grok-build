use super::*;
use crate::views::dashboard::persistence::{DashboardWrite, enqueue, with_test_writer};
use crate::views::dashboard::state::load_persisted_from_path;
use crate::views::dashboard::{DashboardRowId, PersistedRowId, SessionLane};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone, Copy)]
enum LayoutMode {
    Workspace,
    V1,
}

#[derive(Clone, Copy)]
enum RosterSource {
    Leader,
    Local,
}

fn children() -> Vec<crate::app::roster::RosterEntry> {
    ["child-a", "child-b"].map(|id| {
        let mut child = idle_roster_entry(id, id);
        child.session_kind = Some(crate::app::roster::SESSION_KIND_SUBAGENT.into());
        child.parent_session_id = Some("detached-parent".into());
        child
    }).into()
}

fn roster_result(source: RosterSource, sessions: Vec<crate::app::roster::RosterEntry>) -> Action {
    Action::TaskComplete(match source {
        RosterSource::Leader => TaskResult::RosterLoaded { sessions },
        RosterSource::Local => TaskResult::DashboardSessionsLoaded { sessions },
    })
}

fn roster_row(id: &str) -> DashboardRowId {
    DashboardRowId::Roster { session_id: id.into() }
}

fn assert_child_layout(app: &AppView) {
    let dashboard = app.dashboard.as_ref().unwrap();
    assert_eq!(dashboard.pinned, BTreeSet::from([roster_row("child-a"), roster_row("child-b")]));
    assert_eq!(dashboard.reorder, vec![roster_row("child-b"), roster_row("child-a")]);
    assert_eq!(dashboard.child_rows, BTreeMap::from([
        ("child-a".into(), roster_row("child-a")),
        ("child-b".into(), roster_row("child-b")),
    ]));
    let saved = app.dashboard_persisted.as_ref().unwrap();
    assert_eq!(saved.pinned.iter().map(PersistedRowId::to_key).collect::<Vec<_>>(),
        vec!["top:child-a", "top:child-b"]);
    assert_eq!(saved.reorder.iter().map(PersistedRowId::to_key).collect::<Vec<_>>(),
        vec!["top:child-b", "top:child-a"]);
}

fn assert_disk(config: &Path, pins: &[&str], order: &[&str]) {
    let doc: toml_edit::DocumentMut = std::fs::read_to_string(config).unwrap().parse().unwrap();
    assert_eq!(doc["preferences"]["keep"].as_str(), Some("untouched"));
    for (field, expected) in [("pinned", pins), ("reorder", order)] {
        assert_eq!(doc["dashboard"][field].as_array().unwrap().iter()
            .map(|value| value.as_str().unwrap()).collect::<Vec<_>>(), expected);
    }
    let saved = load_persisted_from_path(config).unwrap();
    assert_eq!(saved.pinned.iter().map(PersistedRowId::to_key).collect::<Vec<_>>(), pins);
    assert_eq!(saved.reorder.iter().map(PersistedRowId::to_key).collect::<Vec<_>>(), order);
}

fn persist(effects: Vec<Effect>, lane: SessionLane) {
    let write = match lane {
        SessionLane::Main => {
            let [Effect::PersistDashboard(saved)] = effects.as_slice() else {
                panic!("Main must emit its maintained snapshot: {effects:?}");
            };
            DashboardWrite::Full(saved.clone())
        }
        SessionLane::Subagents => {
            let [Effect::PersistDashboardChildLayout(update)] = effects.as_slice() else {
                panic!("Subagents must emit its scoped layout: {effects:?}");
            };
            assert_eq!(update.scope, BTreeSet::from([
                PersistedRowId::TopLevel { session_id: "child-a".into() },
                PersistedRowId::TopLevel { session_id: "child-b".into() },
            ]));
            DashboardWrite::Child(update.clone())
        }
    };
    enqueue(write).unwrap().blocking_recv().unwrap().unwrap();
}

fn allocated_reopen(mode: LayoutMode, source: RosterSource) {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    std::fs::write(&config, "[dashboard]\npinned = ['top:child-a', 'top:child-b']\nreorder = ['top:child-b', 'top:child-a']\n[preferences]\nkeep = 'untouched'\n").unwrap();
    let mut app = test_app_with_agent();
    app.workspace_dashboard_enabled = matches!(mode, LayoutMode::Workspace);
    app.leader_mode = matches!(source, RosterSource::Leader);
    app.agents.get_mut(&AgentId(0)).unwrap().session.session_id = Some(acp::SessionId::new("root-a"));
    mark_agent_nonempty(&mut app, AgentId(0));
    if matches!(mode, LayoutMode::Workspace) {
        app.workspace_membership.set_snapshot_for_test(workspace_snapshot(vec![member("root-a", "Root A")]));
    }
    app.dashboard_persisted = load_persisted_from_path(&config);
    assert!(app.dashboard.is_none());
    let _ = dispatch(roster_result(source, children()), &mut app);
    let chord = KeyModifiers::CONTROL | KeyModifiers::ALT;
    press(&mut app, KeyCode::Up, chord);
    assert_eq!(app.active_view, ActiveView::AgentDashboard);
    assert_child_layout(&app);
    press(&mut app, KeyCode::Right, chord);
    assert_eq!(app.dashboard.as_ref().unwrap().session_lane(), SessionLane::Subagents);
    let allocation = std::ptr::from_ref(app.dashboard.as_ref().unwrap());
    let membership_key = app.dashboard.as_ref().unwrap().child_rows
        .get_key_value("child-b").unwrap().0.as_ptr();
    let roster_before = app.dashboard_roster().as_ptr();
    let workspace_before = app.workspace_membership.snapshot().cloned();
    let workspace_order = app.workspace_membership.effective_manual_order();

    press(&mut app, KeyCode::Down, chord);
    assert_eq!(app.active_view, ActiveView::Agent(AgentId(0)));
    press(&mut app, KeyCode::Up, chord);
    assert_eq!(app.active_view, ActiveView::AgentDashboard);
    assert_eq!(std::ptr::from_ref(app.dashboard.as_ref().unwrap()), allocation);
    assert_eq!(app.dashboard.as_ref().unwrap().child_rows
        .get_key_value("child-b").unwrap().0.as_ptr(), membership_key);
    assert_eq!(app.dashboard_roster().as_ptr(), roster_before);
    assert_eq!(app.dashboard_roster().iter().map(|entry| entry.session_id.as_str())
        .collect::<Vec<_>>(), vec!["child-a", "child-b"]);
    assert_child_layout(&app);

    with_test_writer(config.clone(), |_| {}, || {
        if matches!(mode, LayoutMode::V1) {
            press(&mut app, KeyCode::Left, chord);
            assert_eq!(app.dashboard.as_ref().unwrap().session_lane(), SessionLane::Main);
            app.dashboard.as_mut().unwrap().focus_row(DashboardRowId::TopLevel(AgentId(0)));
            persist(dispatch(Action::DashboardTogglePin, &mut app), SessionLane::Main);
            assert_disk(&config, &["top:child-a", "top:child-b", "top:root-a"],
                &["top:child-b", "top:child-a"]);
        }
        press(&mut app, KeyCode::Right, chord);
        assert_eq!(app.dashboard.as_ref().unwrap().session_lane(), SessionLane::Subagents);
        app.dashboard.as_mut().unwrap().focus_row(roster_row("child-a"));
        persist(dispatch(Action::DashboardTogglePin, &mut app), SessionLane::Subagents);
        match mode {
            LayoutMode::Workspace => assert_disk(&config, &["top:child-b"], &["top:child-b", "top:child-a"]),
            LayoutMode::V1 => assert_disk(&config, &["top:child-b", "top:root-a"], &["top:child-b", "top:child-a"]),
        }
        assert_eq!(app.dashboard.as_ref().unwrap().reorder, vec![roster_row("child-b"), roster_row("child-a")]);
        persist(dispatch(Action::DashboardReorderUp, &mut app), SessionLane::Subagents);
        match mode {
            LayoutMode::Workspace => assert_disk(&config, &["top:child-b"], &["top:child-a", "top:child-b"]),
            LayoutMode::V1 => assert_disk(&config, &["top:child-b", "top:root-a"], &["top:child-a", "top:child-b"]),
        }
    }).unwrap();
    assert_eq!(app.workspace_membership.snapshot(), workspace_before.as_ref());
    assert_eq!(app.workspace_membership.effective_manual_order(), workspace_order);
    assert_eq!(app.dashboard_roster().as_ptr(), roster_before);
    assert_eq!(app.dashboard_roster().iter().map(|entry| entry.session_id.as_str())
        .collect::<Vec<_>>(), vec!["child-a", "child-b"]);

    // The delayed fetch must not be needed to preserve the reopened layout.
    let _ = dispatch(roster_result(source, children()), &mut app);
    let dashboard = app.dashboard.as_ref().unwrap();
    assert!(!dashboard.pinned.contains(&roster_row("child-a")));
    assert!(dashboard.pinned.contains(&roster_row("child-b")));
    assert_eq!(dashboard.reorder, vec![roster_row("child-a"), roster_row("child-b")]);
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn workspace_allocated_dashboard_reopen_preserves_child_layout_through_fifo_reload() {
    for source in [RosterSource::Leader, RosterSource::Local] {
        allocated_reopen(LayoutMode::Workspace, source);
    }
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn v1_allocated_dashboard_reopen_preserves_children_before_delayed_roster_and_main_save() {
    for source in [RosterSource::Leader, RosterSource::Local] {
        allocated_reopen(LayoutMode::V1, source);
    }
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn allocated_dashboard_gc_uses_roster_liveness_for_unattached_main_rows() {
    for source in [RosterSource::Leader, RosterSource::Local] {
        let mut app = test_app_with_agent();
        app.workspace_dashboard_enabled = false;
        app.leader_mode = matches!(source, RosterSource::Leader);
        app.dashboard_persisted = Some(crate::views::dashboard::PersistedDashboard::defaults());
        mark_agent_nonempty(&mut app, AgentId(0));
        let _ = dispatch(roster_result(source, vec![idle_roster_entry("roster-root", "Roster root")]), &mut app);
        let chord = KeyModifiers::CONTROL | KeyModifiers::ALT;
        press(&mut app, KeyCode::Up, chord);
        let row = roster_row("roster-root");
        app.dashboard.as_mut().unwrap().focus_row(row.clone());
        let _ = dispatch(Action::DashboardTogglePin, &mut app);
        let _ = dispatch(Action::DashboardReorderUp, &mut app);
        assert!(app.agents.find_by_session_id("roster-root").is_none());
        assert!(app.dashboard.as_ref().unwrap().child_rows.is_empty());
        press(&mut app, KeyCode::Down, chord);
        assert_eq!(app.active_view, ActiveView::Agent(AgentId(0)));
        press(&mut app, KeyCode::Up, chord);
        assert_eq!(app.active_view, ActiveView::AgentDashboard);
        assert_eq!(app.dashboard.as_ref().unwrap().pinned, BTreeSet::from([row.clone()]));
        assert_eq!(app.dashboard.as_ref().unwrap().reorder, vec![row]);
        let _ = dispatch(roster_result(source, vec![]), &mut app);
        press(&mut app, KeyCode::Down, chord);
        press(&mut app, KeyCode::Up, chord);
        assert_eq!(app.active_view, ActiveView::AgentDashboard);
        assert!(app.dashboard.as_ref().unwrap().pinned.is_empty());
        assert!(app.dashboard.as_ref().unwrap().reorder.is_empty());
    }
}
