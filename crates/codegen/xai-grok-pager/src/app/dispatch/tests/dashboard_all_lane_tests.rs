//! Dashboard All-lane, scope, and paging dispatch tests, driven through the router.
use super::*;
use crate::app::actions::Direction;

fn app_with_three_named_agents() -> AppView {
    let mut app = test_app();
    app.dashboard_persisted = Some(crate::views::dashboard::PersistedDashboard::defaults());
    let now = Instant::now();
    for (slot, name) in ["alpha", "beta", "gamma"].into_iter().enumerate() {
        let mut agent = crate::app::agent_view::test_fixtures::make_agent();
        agent.display_name = Some(name.to_owned());
        agent.session.session_id = Some(agent_client_protocol::SessionId::new(name));
        agent.last_active_at = Some(now - std::time::Duration::from_secs(slot as u64));
        app.agents.insert(crate::app::agent::AgentId(slot), agent);
    }
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    app
}

fn press_page_key(app: &mut AppView, code: crossterm::event::KeyCode) {
    use crossterm::event::{Event, KeyEvent, KeyModifiers};
    if let crate::app::app_view::InputOutcome::Action(action) =
        app.handle_input(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    {
        let _ = dispatch(action, app);
    }
}

fn paint_dashboard(app: &mut AppView, height: u16) -> String {
    use ratatui::{buffer::Buffer, layout::Rect};
    let area = Rect::new(0, 0, 140, height);
    let mut buffer = Buffer::empty(area);
    crate::views::dashboard::render_dashboard(
        &mut buffer, area, app.dashboard.as_mut().unwrap(), &mut app.agents,
        &app.registry, None, &[], false,
        crate::views::dashboard::WorkspaceRowInputs::default(), None, false, None, None,
    );
    (0..height).map(|y| {
        (0..area.width).map(|x| buffer.cell((x, y)).unwrap().symbol()).collect::<String>()
    }).collect::<Vec<_>>().join("\n")
}

#[test]
fn dashboard_home_names_idle_section_and_end_names_gamma() {
    use crossterm::event::KeyCode;
    use crate::views::dashboard::{DashboardRowId, RowState, SectionKey};
    let mut app = app_with_three_named_agents();
    let state = app.dashboard.as_mut().unwrap();
    state.preview_enabled = false;
    state.focus_row(DashboardRowId::TopLevel(crate::app::agent::AgentId(1)));
    assert!(paint_dashboard(&mut app, 40).contains("Idle 3"));
    press_page_key(&mut app, KeyCode::Home);
    assert_eq!(app.dashboard.as_ref().unwrap().selected_section,
        Some(SectionKey::State(RowState::Idle)));
    assert!(paint_dashboard(&mut app, 40).contains("Idle 3"));
    press_page_key(&mut app, KeyCode::End);
    assert_eq!(app.dashboard.as_ref().unwrap().selected,
        Some(DashboardRowId::TopLevel(crate::app::agent::AgentId(2))));
    assert!(paint_dashboard(&mut app, 40).contains("gamma"));
    app.dashboard.as_mut().unwrap().focus_new_agent_button();
    press_page_key(&mut app, KeyCode::PageUp);
    assert!(app.dashboard.as_ref().unwrap().new_agent_button_focused());
    press_page_key(&mut app, KeyCode::PageDown);
    assert_eq!(app.dashboard.as_ref().unwrap().selected_section,
        Some(SectionKey::State(RowState::Idle)));
}

/// An empty lane keeps the cursor intact: paging keys must not invent rows or drop the
/// `+ New Agent` focus while the list has nothing to show.
#[test]
fn dashboard_page_on_empty_lane_keeps_button_focus() {
    use crate::views::dashboard::PageMove;
    let mut app = test_app();
    app.dashboard_persisted = Some(crate::views::dashboard::PersistedDashboard::defaults());
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    for mv in [
        PageMove::Up,
        PageMove::Down,
        PageMove::Home,
        PageMove::End,
    ] {
        dispatch(Action::DashboardPage(mv), &mut app);
        assert!(
            app.dashboard
                .as_ref()
                .unwrap()
                .actions_focus
                .is_some(),
            "paging in an empty list must leave the button cursor alone ({mv:?})"
        );
    }
}

/// `Ctrl+A`'s dispatch twin (the header prev/next session cycle) now walks the three lanes in
/// both directions instead of flipping between two.
#[test]
fn session_cycle_in_dashboard_walks_all_three_lanes() {
    use crate::views::dashboard::SessionLane;
    let mut app = app_with_three_named_agents();
    let lane = |app: &AppView| app.dashboard.as_ref().unwrap().session_lane();
    assert_eq!(lane(&app), SessionLane::Main);
    dispatch(Action::CycleSessions(Direction::Next), &mut app);
    assert_eq!(lane(&app), SessionLane::Subagents);
    dispatch(Action::CycleSessions(Direction::Next), &mut app);
    assert_eq!(lane(&app), SessionLane::All);
    dispatch(Action::CycleSessions(Direction::Next), &mut app);
    assert_eq!(lane(&app), SessionLane::Main);
    dispatch(Action::CycleSessions(Direction::Prev), &mut app);
    assert_eq!(lane(&app), SessionLane::All);
}

/// The lane-independent row identity lists the same root under one stable id: switching to the
/// All lane keeps the previously selectable root addressable instead of shifting ids.
#[test]
fn all_lane_lists_root_under_stable_identity() {
    use crate::views::dashboard::{DashboardRowId, SessionLane};
    let mut app = app_with_three_named_agents();
    let target = DashboardRowId::TopLevel(crate::app::agent::AgentId(1));
    app.dashboard.as_mut().unwrap().focus_row(target.clone());

    app.dashboard
        .as_mut()
        .unwrap()
        .set_session_lane(SessionLane::All);
    let focusables = crate::app::dispatch::dashboard::dashboard_focusables(&app);
    assert!(
        focusables.iter().any(|f| matches!(f, crate::views::dashboard::Focusable::Row(id) if *id == target)),
        "the All lane must list the root row under its stable identity"
    );
}

#[test]
fn rendered_directory_page_moves_six_named_sessions_in_eighteen_cells() {
    use crossterm::event::KeyCode;
    use crate::views::dashboard::{DashboardRowId, Grouping};
    let mut app = test_app();
    app.dashboard_persisted = Some(crate::views::dashboard::PersistedDashboard::defaults());
    let now = Instant::now();
    for slot in 0..20 {
        let mut agent = crate::app::agent_view::test_fixtures::make_agent();
        let name = format!("page-{slot:02}");
        agent.display_name = Some(name.clone());
        agent.session.session_id = Some(acp::SessionId::new(name));
        agent.last_active_at = Some(now - std::time::Duration::from_secs(slot as u64));
        app.agents.insert(crate::app::agent::AgentId(slot), agent);
    }
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    let state = app.dashboard.as_mut().unwrap();
    state.preview_enabled = false;
    state.grouping = Grouping::Directory;
    press_page_key(&mut app, KeyCode::Home);
    let initial = paint_dashboard(&mut app, 30);
    assert_eq!(app.dashboard.as_ref().unwrap().last_viewport_h, 18);
    assert!(initial.contains("page-00") && initial.contains("page-05"));
    assert!(!initial.contains("page-06"));
    press_page_key(&mut app, KeyCode::PageDown);
    assert_eq!(app.dashboard.as_ref().unwrap().selected,
        Some(DashboardRowId::TopLevel(crate::app::agent::AgentId(6))));
    let next = paint_dashboard(&mut app, 30);
    assert!(next.contains("page-06"));
    assert!(!next.contains("page-00"));
    press_page_key(&mut app, KeyCode::PageUp);
    assert_eq!(app.dashboard.as_ref().unwrap().selected,
        Some(DashboardRowId::TopLevel(crate::app::agent::AgentId(0))));
    assert!(paint_dashboard(&mut app, 30).contains("page-00"));
    press_page_key(&mut app, KeyCode::End);
    assert_eq!(app.dashboard.as_ref().unwrap().selected,
        Some(DashboardRowId::TopLevel(crate::app::agent::AgentId(19))));
    assert!(paint_dashboard(&mut app, 30).contains("page-19"));
}

#[test]
fn mixed_lane_render_names_complete_rows_and_state_headers_without_overrides() {
    use crate::app::agent::AgentId;
    use crate::app::agent_view::{AgentRole, ChildLink};
    use crate::views::dashboard::{Grouping, SessionLane};
    let mut app = test_app();
    app.dashboard_persisted = Some(crate::views::dashboard::PersistedDashboard::defaults());
    let now = Instant::now();
    for (slot, name, age, running, child) in [
        (5, "IdleChildOld", 40, false, true),
        (3, "IdleRootOld", 30, false, false),
        (0, "RunRoot", 50, true, false),
        (4, "IdleChildNew", 20, false, true),
        (2, "IdleRootNew", 10, false, false),
        (1, "RunChild", 60, true, true),
    ] {
        let mut agent = crate::app::agent_view::test_fixtures::make_agent();
        agent.display_name = Some(name.into());
        agent.session.session_id = Some(acp::SessionId::new(name));
        agent.last_active_at = Some(now - std::time::Duration::from_secs(age));
        if running {
            agent.session.bg_tasks.insert("task".into(),
                crate::app::agent_test_fixtures::running_bg_task("task", false));
        }
        if child {
            agent.role = AgentRole::Child(ChildLink {
                parent: Some(AgentId(0)),
                parent_session_id: acp::SessionId::new("RunRoot"),
                subagent_id: name.into(), started_at: now,
            });
        }
        app.agents.insert(AgentId(slot), agent);
    }
    app.active_view = ActiveView::AgentDashboard;
    ensure_dashboard_state(&mut app);
    let names = ["RunRoot", "RunChild", "IdleRootNew", "IdleChildNew", "IdleRootOld", "IdleChildOld"];
    for (lane, expected, running_count, idle_count) in [
        (SessionLane::Main, vec!["RunRoot", "IdleRootNew", "IdleRootOld"], 1, 2),
        (SessionLane::Subagents, vec!["RunChild", "IdleChildNew", "IdleChildOld"], 1, 2),
        (SessionLane::All, names.to_vec(), 2, 4),
    ] {
        for grouping in [Grouping::State, Grouping::Directory] {
            let state = app.dashboard.as_mut().unwrap();
            state.set_session_lane(lane);
            state.grouping = grouping;
            state.preview_enabled = false;
            state.focus_new_agent_button();
            let painted = paint_dashboard(&mut app, 60);
            let visible: Vec<_> = painted.lines().flat_map(|line| {
                names.iter().copied().filter(move |name| line.contains(name))
            }).collect();
            assert_eq!(visible, expected, "{lane:?} {grouping:?}");
            let headers: Vec<_> = painted.lines().filter_map(|line| {
                if line.contains(&format!("Working {running_count}")) { Some("Working") }
                else if line.contains(&format!("Idle {idle_count}")) { Some("Idle") }
                else { None }
            }).collect();
            let expected_headers = match grouping {
                Grouping::State => vec!["Working", "Idle"],
                Grouping::Directory => vec![],
            };
            assert_eq!(headers, expected_headers, "{lane:?} {grouping:?}");
            press_page_key(&mut app, crossterm::event::KeyCode::Home);
            match grouping {
                Grouping::State => assert_eq!(app.dashboard.as_ref().unwrap().selected_section,
                    Some(crate::views::dashboard::SectionKey::State(crate::views::dashboard::RowState::Working))),
                Grouping::Directory => {
                    let first = match lane {
                        SessionLane::Main | SessionLane::All => AgentId(0),
                        SessionLane::Subagents => AgentId(1),
                    };
                    assert_eq!(app.dashboard.as_ref().unwrap().selected,
                        Some(crate::views::dashboard::DashboardRowId::TopLevel(first)));
                }
            }
            assert!(paint_dashboard(&mut app, 60).contains(expected[0]));
            press_page_key(&mut app, crossterm::event::KeyCode::End);
            let last = match lane {
                SessionLane::Main => AgentId(3),
                SessionLane::Subagents | SessionLane::All => AgentId(5),
            };
            assert_eq!(app.dashboard.as_ref().unwrap().selected,
                Some(crate::views::dashboard::DashboardRowId::TopLevel(last)));
            assert!(paint_dashboard(&mut app, 60).contains(*expected.last().unwrap()));
        }
    }
}
