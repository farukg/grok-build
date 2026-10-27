//! Session-switch restoration of pane focus, selection, viewport and follow state, and
//! the first-arrow anchoring after returning to a session. Driven through the real
//! `AppView::handle_input` / `dispatch` entry points.

use super::tests::test_app_with_agent;
use super::{ActiveView, AppView};
use crate::app::actions::{Action, Direction};
use crate::app::agent::AgentId;
use crate::app::agent_view::{AgentPane, AgentView};
use crate::app::app_view::InputOutcome;
use crate::scrollback::block::RenderBlock;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

fn plain_key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn prompt_session(sid: &str) -> AgentView {
    let mut view =
        crate::app::agent_view::test_agent_view(Some(sid), std::path::PathBuf::from("/tmp"));
    view.session.id = AgentId(1);
    view.set_active_pane(AgentPane::Prompt, true);
    view
}

fn two_session_app() -> AppView {
    let mut app = test_app_with_agent();
    let second = prompt_session("session-b");
    app.agents.insert(AgentId(1), second);
    app
}

fn seed_near_tail_transcript(app: &mut AppView, id: AgentId) -> usize {
    let agent = app.agents.get_mut(&id).expect("session present");
    for i in 0..12 {
        agent
            .scrollback
            .push_block(RenderBlock::agent_message(format!("message {i}\nline two")));
    }
    agent.scrollback.prepare_layout(80, 10);
    agent.scrollback.goto_bottom();
    agent.scrollback.clear_selection();
    agent.scrollback.scroll_up(6);
    agent.scrollback.scroll_offset()
}

fn switch(app: &mut AppView, action: Action) {
    app.capture_sidebar_chrome();
    let _ = crate::app::dispatch::dispatch(action, app);
    app.reconcile_sidebar_chrome();
}

#[test]
fn switching_sessions_propagates_visible_focus_without_importing_selection() {
    let mut app = two_session_app();
    seed_near_tail_transcript(&mut app, AgentId(0));
    let remembered = seed_near_tail_transcript(&mut app, AgentId(1));
    app.agents.get_mut(&AgentId(0)).unwrap().scrollback.set_selected(Some(10));
    app.reconcile_sidebar_chrome();

    switch(&mut app, Action::CycleSessions(Direction::Next));

    assert_eq!(app.active_view, ActiveView::Agent(AgentId(1)));
    let agent = app.agents.get(&AgentId(1)).unwrap();
    assert_eq!(agent.active_pane, AgentPane::Scrollback);
    assert_eq!(agent.scrollback.selected(), None);
    assert_eq!(agent.scrollback.scroll_offset(), remembered);
    assert!(!agent.scrollback.is_follow_mode());
}

#[test]
fn session_switch_preserves_pane_focus_selection_viewport_and_follow() {
    let mut app = two_session_app();
    seed_near_tail_transcript(&mut app, AgentId(0));
    seed_near_tail_transcript(&mut app, AgentId(1));
    let a = app.agents.get_mut(&AgentId(0)).expect("session A present");
    a.scrollback.set_selected(Some(10));
    let offset = a.scrollback.scroll_offset();
    assert!(!a.scrollback.is_follow_mode());
    app.reconcile_sidebar_chrome();

    switch(&mut app, Action::CycleSessions(Direction::Next));
    switch(&mut app, Action::CycleSessions(Direction::Next));

    assert_eq!(app.active_view, ActiveView::Agent(AgentId(0)));
    let agent = app.agents.get(&AgentId(0)).unwrap();
    assert_eq!(agent.active_pane, AgentPane::Scrollback);
    assert_eq!(agent.scrollback.selected(), Some(10));
    assert_eq!(agent.scrollback.scroll_offset(), offset);
    assert!(!agent.scrollback.is_follow_mode());
}

#[test]
fn session_switch_preserves_follow_state_of_a_following_session() {
    let mut app = two_session_app();
    let a = app.agents.get_mut(&AgentId(0)).expect("session A present");
    a.scrollback
        .push_block(RenderBlock::agent_message("streaming answer"));
    a.scrollback.prepare_layout(80, 10);
    a.scrollback.enable_follow_mode();
    assert!(a.scrollback.is_follow_mode());
    app.reconcile_sidebar_chrome();

    switch(&mut app, Action::CycleSessions(Direction::Next));
    switch(&mut app, Action::CycleSessions(Direction::Next));

    let agent = app.agents.get(&AgentId(0)).unwrap();
    assert!(
        agent.scrollback.is_follow_mode(),
        "a following session must return still following"
    );
}

#[test]
fn first_down_after_return_anchors_to_the_remembered_viewport() {
    let mut app = two_session_app();
    let remembered = seed_near_tail_transcript(&mut app, AgentId(0));
    assert!(remembered > 0);
    app.reconcile_sidebar_chrome();

    switch(&mut app, Action::CycleSessions(Direction::Next));
    switch(&mut app, Action::CycleSessions(Direction::Next));

    let outcome = app.handle_input(&plain_key(KeyCode::Down));
    assert!(
        matches!(outcome, InputOutcome::Action(Action::SelectNext)),
        "Down with scrollback focus routes through the registry, got {outcome:?}"
    );
    let _ = crate::app::dispatch::dispatch(Action::SelectNext, &mut app);

    let agent = app.agents.get(&AgentId(0)).unwrap();
    let sel = agent.scrollback.selected().expect("Down selected an entry");
    assert!(sel > 0, "first Down must not select entry zero, got {sel}");
    let delta = agent.scrollback.scroll_offset().abs_diff(remembered);
    assert!(
        delta <= 10,
        "first Down must not teleport the viewport (moved {delta} rows)"
    );
}

#[test]
fn first_up_after_return_anchors_to_the_remembered_viewport() {
    let mut app = two_session_app();
    let remembered = seed_near_tail_transcript(&mut app, AgentId(0));
    app.reconcile_sidebar_chrome();

    switch(&mut app, Action::CycleSessions(Direction::Next));
    switch(&mut app, Action::CycleSessions(Direction::Next));

    let outcome = app.handle_input(&plain_key(KeyCode::Up));
    assert!(
        matches!(outcome, InputOutcome::Action(Action::SelectPrev)),
        "Up with scrollback focus routes through the registry, got {outcome:?}"
    );
    let _ = crate::app::dispatch::dispatch(Action::SelectPrev, &mut app);

    let agent = app.agents.get(&AgentId(0)).unwrap();
    let sel = agent.scrollback.selected().expect("Up selected an entry");
    assert!(sel > 0, "first Up must not fall back to entry zero, got {sel}");
    assert!(
        agent.scrollback.scroll_offset().abs_diff(remembered) <= 10,
        "first Up must not teleport the viewport"
    );
}


#[test]
fn returning_to_a_nonselectable_window_uses_its_directional_boundary() {
    for code in [KeyCode::Down, KeyCode::Up] {
        let mut app = two_session_app();
        seed_near_tail_transcript(&mut app, AgentId(0));
        let system = {
            let a = app.agents.get_mut(&AgentId(0)).unwrap();
            let system = a.scrollback.len();
            a.scrollback.push_block(RenderBlock::system("system row\n".repeat(100)));
            a.scrollback.push_block(RenderBlock::agent_message("after system"));
            a.scrollback.prepare_layout(80, 10);
            a.scrollback.scroll_to_entry_top(system);
            a.scrollback.scroll_down(5);
            a.scrollback.clear_selection();
            system
        };
        app.reconcile_sidebar_chrome();
        switch(&mut app, Action::CycleSessions(Direction::Next));
        switch(&mut app, Action::CycleSessions(Direction::Next));
        let action = match code {
            KeyCode::Down => Action::SelectNext,
            KeyCode::Up => Action::SelectPrev,
            _ => unreachable!(),
        };
        let outcome = app.handle_input(&plain_key(code));
        assert!(matches!(
            (code, outcome),
            (KeyCode::Down, InputOutcome::Action(Action::SelectNext))
                | (KeyCode::Up, InputOutcome::Action(Action::SelectPrev))
        ));
        let _ = crate::app::dispatch::dispatch(action, &mut app);
        let a = app.agents.get(&AgentId(0)).unwrap();
        assert_eq!(a.scrollback.selected(), Some(if code == KeyCode::Down { system + 1 } else { system - 1 }));
        assert!(a.scrollback.scroll_offset() > 0);
        assert_eq!(a.active_pane, AgentPane::Scrollback);
    }
}

#[test]
fn switching_to_an_unlaid_out_session_keeps_range_fallback_local() {
    let mut app = two_session_app();
    seed_near_tail_transcript(&mut app, AgentId(0));
    let b = app.agents.get_mut(&AgentId(1)).unwrap();
    b.scrollback.push_block(RenderBlock::system("not selectable"));
    b.scrollback.push_block(RenderBlock::agent_message("first selectable"));
    app.reconcile_sidebar_chrome();
    switch(&mut app, Action::CycleSessions(Direction::Next));
    assert_eq!(app.agents.get(&AgentId(1)).unwrap().scrollback.selected(), None);
    assert!(matches!(app.handle_input(&plain_key(KeyCode::Down)), InputOutcome::Action(Action::SelectNext)));
    let _ = crate::app::dispatch::dispatch(Action::SelectNext, &mut app);
    let b = app.agents.get(&AgentId(1)).unwrap();
    assert_eq!(b.scrollback.selected(), Some(1));
    assert_eq!(b.scrollback.scroll_offset(), 0);
}


#[test]
fn rendered_session_switch_keeps_target_navigation_after_resize() {
    for selected in [None, Some(48)] {
        let mut app = two_session_app();
        {
            let a = app.agents.get_mut(&AgentId(0)).unwrap();
            for i in 0..50 {
                a.scrollback.push_block(RenderBlock::agent_message(format!("message {i}: {}", "wrapped content ".repeat(10))));
            }
        }
        let (mut terminal, _frames) = crate::test_util::test_terminal_in(ratatui::layout::Rect::new(0, 0, 120, 30));
        app.draw(&mut terminal);
        let a = app.agents.get_mut(&AgentId(0)).unwrap();
        a.scrollback.goto_bottom();
        a.scrollback.scroll_up(6);
        a.scrollback.set_selected(selected);
        let offset = a.scrollback.scroll_offset();
        assert!(offset > 0);
        app.capture_sidebar_chrome();
        switch(&mut app, Action::CycleSessions(Direction::Next));
        switch(&mut app, Action::CycleSessions(Direction::Next));
        let a = app.agents.get(&AgentId(0)).unwrap();
        assert_eq!(a.scrollback.selected(), selected);
        assert_eq!(a.scrollback.scroll_offset(), offset);
        assert!(!a.scrollback.is_follow_mode());
        let (mut resized, _frames) = crate::test_util::test_terminal_in(ratatui::layout::Rect::new(0, 0, 80, 24));
        app.draw(&mut resized);
        assert!(matches!(app.handle_input(&plain_key(KeyCode::Up)), InputOutcome::Action(Action::SelectPrev)));
        let _ = crate::app::dispatch::dispatch(Action::SelectPrev, &mut app);
        let a = app.agents.get(&AgentId(0)).unwrap();
        assert!(a.scrollback.selected().unwrap() > 0);
        assert!(a.scrollback.scroll_offset() > 0);
        assert!(!a.scrollback.is_follow_mode());
    }
}
