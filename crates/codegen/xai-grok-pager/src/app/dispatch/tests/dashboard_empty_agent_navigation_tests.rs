use super::*;
use crate::views::dashboard::{DashboardRowId, Filter, RowState, SessionLane, SessionScope};

fn navigation_app(workspace: bool) -> AppView {
    let mut app = test_app_with_agent();
    mark_agent_nonempty(&mut app, AgentId(0));
    app.agents.get_mut(&AgentId(0)).unwrap().last_active_at = Some(Instant::now());
    app.dashboard_persisted = Some(crate::views::dashboard::PersistedDashboard::defaults());
    app.workspace_dashboard_enabled = workspace;
    if workspace {
        app.workspace_membership.set_snapshot_for_test(workspace_snapshot(vec![member(
            "test-session", "Existing",
        )]));
    }
    ensure_dashboard_state(&mut app);
    app.dashboard.as_mut().unwrap().session_scope = SessionScope::All;
    app.dashboard.as_mut().unwrap().cwd = app.cwd.clone();
    app
}

fn create_empty(app: &mut AppView) -> AgentId {
    app.active_view = ActiveView::AgentDashboard;
    let effects = dispatch(Action::DashboardCreateNewAgentWithDetail, app);
    let id = effects.iter().find_map(|effect| match effect {
        Effect::CreateSession { agent_id, .. } => Some(*agent_id),
        _ => None,
    }).expect("dashboard creates an agent");
    let agent = app.agents.get(&id).unwrap();
    assert!(agent.session.session_id.is_none());
    assert!(agent.session.pending_prompts.is_empty());
    assert!(crate::views::dashboard::peek::extract_first_user_message(agent).is_none());
    assert_eq!(app.active_view, ActiveView::Agent(id));
    id
}

fn bind(app: &mut AppView, id: AgentId, session_id: &str) {
    let _ = dispatch(Action::TaskComplete(TaskResult::SessionCreated {
        agent_id: id,
        session_id: acp::SessionId::new(session_id.to_owned()),
        models: None,
        modes: None,
    }), app);
}

fn assert_position(app: &mut AppView, id: AgentId, index: usize, total: usize) {
    assert_eq!(app.active_view, ActiveView::Agent(id));
    assert_eq!(crate::app::dispatch::session_cycle_position(app, id), Some((index, total)));
    let row = painted_switcher_row(app).expect("session header switcher is painted");
    assert!(row.contains(&format!("{index}/{total}")), "header: {row}");
}

fn cycle(app: &mut AppView, code: KeyCode, target: AgentId) {
    let outcome = app.handle_input(&Event::Key(KeyEvent::new(
        code, KeyModifiers::CONTROL | KeyModifiers::ALT,
    )));
    let InputOutcome::Action(action @ Action::CycleSessions(_)) = outcome else {
        panic!("session chord must route to cycling: {outcome:?}");
    };
    let _ = dispatch(action, app);
    assert_eq!(app.active_view, ActiveView::Agent(target));
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn empty_agents_join_header_and_bidirectional_keyboard_cycle_before_binding() {
    for workspace in [false, true] {
        let mut app = navigation_app(workspace);
        let first = create_empty(&mut app);
        assert_position(&mut app, first, 2, 2);
        let second = create_empty(&mut app);
        assert_eq!(dashboard_row_order(&app), vec![
            DashboardRowId::TopLevel(AgentId(0)),
            DashboardRowId::TopLevel(first),
            DashboardRowId::TopLevel(second),
        ]);
        assert_position(&mut app, second, 3, 3);
        cycle(&mut app, KeyCode::Right, AgentId(0));
        assert_position(&mut app, AgentId(0), 1, 3);
        cycle(&mut app, KeyCode::Right, first);
        cycle(&mut app, KeyCode::Right, second);
        cycle(&mut app, KeyCode::Left, first);
        cycle(&mut app, KeyCode::Left, AgentId(0));
        cycle(&mut app, KeyCode::Left, second);
        assert_position(&mut app, second, 3, 3);
    }
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn reverse_identity_binding_keeps_empty_row_order_selection_and_header() {
    for workspace in [false, true] {
        let mut app = navigation_app(workspace);
        let first = create_empty(&mut app);
        let second = create_empty(&mut app);
        let expected = vec![DashboardRowId::TopLevel(AgentId(0)),
            DashboardRowId::TopLevel(first), DashboardRowId::TopLevel(second)];
        bind(&mut app, second, "a-second");
        assert_eq!(dashboard_row_order(&app), expected);
        assert_position(&mut app, second, 3, 3);
        bind(&mut app, first, "z-first");
        assert_eq!(dashboard_row_order(&app), expected);
        assert_eq!(app.dashboard.as_ref().unwrap().selected, Some(DashboardRowId::TopLevel(second)));
        assert_eq!(app.dashboard.as_ref().unwrap().attached_agent, Some(second));
        if workspace {
            app.workspace_membership.set_snapshot_for_test(workspace_snapshot(vec![
                member("test-session", "Existing"), member("z-first", ""), member("a-second", ""),
            ]));
            app.invalidate_session_cycle();
            assert_eq!(dashboard_row_order(&app), expected);
        }
        assert_position(&mut app, second, 3, 3);
        cycle(&mut app, KeyCode::Left, first);
        cycle(&mut app, KeyCode::Right, second);
    }
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn nonempty_explicit_agents_keep_the_existing_session_identity_tie_break() {
    for workspace in [false, true] {
        let mut app = navigation_app(workspace);
        let first = create_empty(&mut app);
        let second = create_empty(&mut app);
        app.agents.get_mut(&first).unwrap().display_name = Some("First".into());
        app.agents.get_mut(&second).unwrap().display_name = Some("Second".into());
        bind(&mut app, second, "a-second");
        bind(&mut app, first, "z-first");
        assert_eq!(dashboard_row_order(&app), vec![
            DashboardRowId::TopLevel(AgentId(0)),
            DashboardRowId::TopLevel(second),
            DashboardRowId::TopLevel(first),
        ]);
        assert_position(&mut app, second, 2, 3);
        cycle(&mut app, KeyCode::Right, first);
        cycle(&mut app, KeyCode::Left, second);
    }
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn dismiss_reopen_and_close_empty_agent_update_the_shared_navigation() {
    for workspace in [false, true] {
        let mut app = navigation_app(workspace);
        let first = create_empty(&mut app);
        let second = create_empty(&mut app);
        let _ = dispatch(Action::DismissActiveSession, &mut app);
        assert_position(&mut app, AgentId(0), 1, 2);
        cycle(&mut app, KeyCode::Left, first);
        let _ = dispatch(Action::DashboardAttach(DashboardRowId::TopLevel(second)), &mut app);
        assert_position(&mut app, second, 3, 3);
        let _ = dispatch(Action::DashboardOverlayStop, &mut app);
        assert!(!app.agents.contains_key(&second));
        let _ = dispatch(Action::DashboardAttach(DashboardRowId::TopLevel(first)), &mut app);
        assert_position(&mut app, first, 2, 2);
        cycle(&mut app, KeyCode::Right, AgentId(0));
        cycle(&mut app, KeyCode::Left, first);
    }
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn empty_root_does_not_import_roster_rows_or_children_into_root_navigation() {
    for workspace in [false, true] {
        let mut app = navigation_app(workspace);
        let root = create_empty(&mut app);
        let mut child = crate::app::agent_view::test_fixtures::make_agent();
        child.display_name = Some("Child".into());
        child.role = crate::app::agent_view::AgentRole::Child(crate::app::agent_view::ChildLink {
            parent: Some(AgentId(0)), parent_session_id: acp::SessionId::new("test-session"),
            subagent_id: "worker".into(), started_at: Instant::now(),
        });
        let child_id = AgentId(20);
        app.agents.insert(child_id, child);
        let mut sibling = crate::app::agent_view::test_fixtures::make_agent();
        sibling.role = crate::app::agent_view::AgentRole::Child(crate::app::agent_view::ChildLink {
            parent: Some(AgentId(0)), parent_session_id: acp::SessionId::new("test-session"),
            subagent_id: "worker-2".into(), started_at: Instant::now(),
        });
        let sibling_id = AgentId(21);
        app.agents.insert(sibling_id, sibling);
        app.agents.insert(AgentId(22), crate::app::agent_view::test_fixtures::make_agent());
        app.dashboard_local_sessions.push(idle_roster_entry("foreign", "Foreign history"));
        app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::All);
        app.invalidate_session_cycle();
        assert_position(&mut app, root, 2, 2);
        cycle(&mut app, KeyCode::Right, AgentId(0));
        cycle(&mut app, KeyCode::Left, root);
        switch_to_agent(&mut app, child_id, SwitchCause::Navigate);
        assert_position(&mut app, child_id, 1, 2);
        cycle(&mut app, KeyCode::Right, sibling_id);
        assert_position(&mut app, sibling_id, 2, 2);
        cycle(&mut app, KeyCode::Left, child_id);
    }
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn empty_worktree_agent_joins_cycle_despite_a_previous_text_filter() {
    let mut app = navigation_app(false);
    app.cwd_has_git_ancestor = true;
    app.active_view = ActiveView::AgentDashboard;
    let dashboard = app.dashboard.as_mut().unwrap();
    dashboard.dispatch_worktree = true;
    dashboard.filter = Filter::Substring("no matching session".into());
    let _ = dispatch(Action::DashboardCreateNewAgentWithDetail, &mut app);
    let effects = dispatch(Action::DashboardConfirmWorktree { label: None }, &mut app);
    let id = effects.iter().find_map(|effect| match effect {
        Effect::CreateWorktreeSession { agent_id, .. } => Some(*agent_id),
        _ => None,
    }).expect("worktree placeholder");
    assert!(app.agents.get(&id).unwrap().session.session_id.is_none());
    assert_position(&mut app, id, 2, 2);
    cycle(&mut app, KeyCode::Right, AgentId(0));
    cycle(&mut app, KeyCode::Left, id);
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn workspace_scope_and_filter_still_bound_empty_agent_navigation() {
    let mut app = navigation_app(true);
    let root = create_empty(&mut app);
    app.agents.get_mut(&AgentId(0)).unwrap().session.cwd = "/elsewhere".into();
    app.dashboard.as_mut().unwrap().session_scope = SessionScope::Cwd;
    app.invalidate_session_cycle();
    assert_eq!(dashboard_row_order(&app), vec![DashboardRowId::TopLevel(root)]);
    assert_eq!(crate::app::dispatch::session_cycle_position(&mut app, root), None);
    cycle(&mut app, KeyCode::Right, root);
    app.dashboard.as_mut().unwrap().session_scope = SessionScope::All;
    app.dashboard.as_mut().unwrap().filter = Filter::State(RowState::Working);
    app.invalidate_session_cycle();
    assert!(dashboard_row_order(&app).is_empty());
    cycle(&mut app, KeyCode::Left, root);
    app.dashboard.as_mut().unwrap().filter = Filter::None;
    app.invalidate_session_cycle();
    assert_position(&mut app, root, 2, 2);
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn creating_empty_agent_preserves_running_entry_order_through_binding() {
    let mut app = navigation_app(false);
    app.agents.get_mut(&AgentId(0)).unwrap().session.enqueue_prompt("first run".into());
    let empty = create_empty(&mut app);
    let mut newer = crate::app::agent_view::test_fixtures::make_agent();
    newer.display_name = Some("New run".into());
    newer.session.enqueue_prompt("second run".into());
    let newer_id = AgentId(30);
    app.agents.insert(newer_id, newer);
    let _ = dispatch(Action::DashboardAttach(DashboardRowId::TopLevel(empty)), &mut app);
    let expected = vec![DashboardRowId::TopLevel(newer_id),
        DashboardRowId::TopLevel(AgentId(0)), DashboardRowId::TopLevel(empty)];
    assert_eq!(dashboard_row_order(&app), expected);
    app.agents.get_mut(&AgentId(0)).unwrap().last_active_at = Some(Instant::now());
    bind(&mut app, empty, "bound-empty");
    assert_eq!(dashboard_row_order(&app), expected);
    assert_position(&mut app, empty, 3, 3);
    cycle(&mut app, KeyCode::Right, newer_id);
    cycle(&mut app, KeyCode::Left, empty);
}

#[serial_test::serial(GROK_AGENT_DASHBOARD)]
#[test]
fn explicit_empty_workspace_agent_is_adopted_after_binding() {
    let (_temp, store) = temp_store();
    let mut app = ready_workspace_app(navigation_app(false), store);
    let id = create_empty(&mut app);
    bind(&mut app, id, "empty-created");
    let effects = crate::app::workspace_sync::drain(&mut app);
    assert!(matches!(effects.as_slice(), [Effect::WriteWorkspace {
        mutation: WorkspaceMutation::Upsert(members), ..
    }] if members.iter().any(|member| member.key.session_id.as_ref() == "empty-created")));
}
