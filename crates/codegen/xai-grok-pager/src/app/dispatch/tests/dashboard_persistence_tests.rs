use super::*;
use crate::views::dashboard::{DashboardRowId, PersistedRowId};
use crate::views::dashboard::state::load_persisted_from_path;
use std::path::Path;

fn fresh_main(config: &Path) -> AppView {
    let mut app = test_app_with_agent();
    app.workspace_dashboard_enabled = false;
    app.leader_mode = true;
    let first = app.agents.get_mut(&AgentId(0)).unwrap();
    first.session.session_id = Some(acp::SessionId::new("root-a"));
    first.display_name = Some("Root A".into());
    let mut second = crate::app::agent_view::test_fixtures::make_agent();
    second.session.session_id = Some(acp::SessionId::new("root-b"));
    second.display_name = Some("Root B".into());
    app.agents.insert(AgentId(1), second);
    app.dashboard_persisted = load_persisted_from_path(config);
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    app
}

fn persist_action(app: &mut AppView, config: &Path, id: AgentId, action: Action) {
    app.dashboard.as_mut().unwrap().focus_row(DashboardRowId::TopLevel(id));
    let effects = dispatch(action, app);
    let [Effect::PersistDashboard(saved)] = effects.as_slice() else {
        panic!("Main v1 must emit its maintained snapshot: {effects:?}");
    };
    crate::views::dashboard::persistence::with_test_writer(config.to_path_buf(), |_| {}, || {
        crate::views::dashboard::persistence::enqueue(
            crate::views::dashboard::persistence::DashboardWrite::Full(saved.clone()),
        ).unwrap().blocking_recv().unwrap().unwrap();
    }).unwrap();
    assert_eq!(load_persisted_from_path(config).unwrap().pinned, saved.pinned);
}

fn discover_children(app: &mut AppView) {
    let sessions = ["unseen-a", "unseen-b"].map(|id| {
        let mut child = idle_roster_entry(id, id);
        child.session_kind = Some(crate::app::roster::SESSION_KIND_SUBAGENT.into());
        child.parent_session_id = Some("detached-parent".into());
        child
    }).into();
    let _ = dispatch(Action::TaskComplete(TaskResult::RosterLoaded { sessions }), app);
    let dashboard = app.dashboard.as_ref().unwrap();
    assert!(dashboard.pinned.contains(&DashboardRowId::Roster { session_id: "unseen-a".into() }));
    assert!(dashboard.pinned.contains(&DashboardRowId::Roster { session_id: "unseen-b".into() }));
    assert_eq!(dashboard.reorder.iter().filter_map(|id| match id {
        DashboardRowId::Roster { session_id } => Some(session_id.as_str()),
        DashboardRowId::TopLevel(_) | DashboardRowId::Workspace { .. } => None,
    }).collect::<Vec<_>>(), vec!["unseen-b", "unseen-a"]);
}

fn layout(config: &Path, field: &str) -> Vec<String> {
    let doc: toml_edit::DocumentMut = std::fs::read_to_string(config).unwrap().parse().unwrap();
    assert_eq!(doc["preferences"]["keep"].as_str(), Some("untouched"));
    doc["dashboard"][field].as_array().unwrap().iter()
        .map(|value| value.as_str().unwrap().to_owned()).collect()
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn main_pin_and_reorder_preserve_undiscovered_and_later_omitted_child_layout() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    std::fs::write(&config, "[dashboard]\npinned = ['top:unseen-a', 'top:unseen-b']\nreorder = ['top:unseen-b', 'top:root-b', 'top:unseen-a', 'top:root-a']\n[preferences]\nkeep = 'untouched'\n").unwrap();
    let mut app = fresh_main(&config);
    assert!(app.leader_roster.is_empty());
    persist_action(&mut app, &config, AgentId(0), Action::DashboardTogglePin);
    persist_action(&mut app, &config, AgentId(0), Action::DashboardReorderUp);
    assert_eq!(layout(&config, "pinned"), vec!["top:root-a", "top:unseen-a", "top:unseen-b"]);
    assert_eq!(layout(&config, "reorder"), vec!["top:unseen-b", "top:root-a", "top:unseen-a", "top:root-b"]);
    let saved = app.dashboard_persisted.as_ref().unwrap();
    assert!(saved.pinned.contains(&PersistedRowId::TopLevel { session_id: "unseen-a".into() }));
    assert!(saved.pinned.contains(&PersistedRowId::TopLevel { session_id: "unseen-b".into() }));
    drop(app);
    let mut reopened = fresh_main(&config);
    discover_children(&mut reopened);
    let _ = dispatch(Action::TaskComplete(TaskResult::RosterLoaded { sessions: vec![] }), &mut reopened);
    assert!(reopened.dashboard.as_ref().unwrap().child_rows.is_empty());
    persist_action(&mut reopened, &config, AgentId(1), Action::DashboardTogglePin);
    persist_action(&mut reopened, &config, AgentId(0), Action::DashboardTogglePin);
    persist_action(&mut reopened, &config, AgentId(1), Action::DashboardReorderUp);
    assert_eq!(layout(&config, "pinned"), vec!["top:root-b", "top:unseen-a", "top:unseen-b"]);
    assert_eq!(layout(&config, "reorder"), vec!["top:unseen-b", "top:root-b", "top:unseen-a", "top:root-a"]);
    drop(reopened);
    let mut reopened = fresh_main(&config);
    discover_children(&mut reopened);
}
