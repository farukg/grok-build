//! Modal keyboard regressions through application input and action dispatch.
use super::*;
use crate::app::app_view::InputOutcome;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

fn press(app: &mut AppView, code: KeyCode) {
    let outcome = app.handle_input(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    if let InputOutcome::Action(action) = outcome {
        let _ = dispatch(action, app);
    }
}

fn history_app() -> AppView {
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.prompt.set_text("draft stays here");
    agent.scrollback.push_block(RenderBlock::agent_message("viewed answer"));
    agent.scrollback.set_selected(Some(0));
    agent.set_active_pane(ActivePane::Scrollback, true);
    app
}

#[test]
fn history_enter_opens_viewer_without_a_second_enter_pasting() {
    let mut app = history_app();
    press(&mut app, KeyCode::Enter);
    assert!(test_agent(&app, AgentId(0)).block_viewer.is_some());
    press(&mut app, KeyCode::Enter);
    let agent = test_agent(&app, AgentId(0));
    assert!(agent.block_viewer.is_some());
    assert_eq!(agent.prompt.text(), "draft stays here");
    assert!(agent.prompt.textarea.elements().is_empty());
}

#[test]
fn history_viewer_y_routes_copy_without_changing_the_composer() {
    let mut app = history_app();
    press(&mut app, KeyCode::Enter);
    let clipboard = crate::test_util::RecordingClipboard::default();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let viewer = agent.block_viewer.as_mut().unwrap();
    viewer.prepare_for_test(ratatui::layout::Rect::new(0, 0, 80, 24));
    viewer.list_state.set_clipboard_provider(Box::new(clipboard.clone()));
    let entry_id = viewer.entry_id;
    press(&mut app, KeyCode::Char('y'));
    let agent = test_agent(&app, AgentId(0));
    assert_eq!(clipboard.writes(), vec!["viewed answer".to_owned()]);
    let viewer = agent.block_viewer.as_ref().unwrap();
    assert!(viewer.list_state.copy_toast_active());
    assert_eq!(viewer.entry_id, entry_id);
    assert_eq!(agent.prompt.text(), "draft stays here");
}

#[test]
fn escape_unwinds_menu_above_history_viewer_then_focuses_input() {
    let mut app = history_app();
    press(&mut app, KeyCode::Enter);
    let viewer_id = test_agent(&app, AgentId(0)).block_viewer.as_ref().unwrap().entry_id;
    let _ = dispatch(Action::OpenSettings, &mut app);
    assert!(test_agent(&app, AgentId(0)).active_modal.is_some());
    press(&mut app, KeyCode::Esc);
    let agent = test_agent(&app, AgentId(0));
    assert!(agent.active_modal.is_none());
    assert_eq!(agent.block_viewer.as_ref().unwrap().entry_id, viewer_id);
    assert_eq!(agent.active_pane, ActivePane::Scrollback);
    press(&mut app, KeyCode::Esc);
    let agent = test_agent(&app, AgentId(0));
    assert!(agent.block_viewer.is_none());
    assert_eq!(agent.active_pane, ActivePane::Prompt);
    assert_eq!(agent.prompt.text(), "draft stays here");
    press(&mut app, KeyCode::Tab);
    assert_eq!(test_agent(&app, AgentId(0)).active_pane, ActivePane::Scrollback);
    press(&mut app, KeyCode::Tab);
    assert_eq!(test_agent(&app, AgentId(0)).active_pane, ActivePane::Prompt);
}

#[test]
fn line_viewer_above_agents_menu_owns_enter_and_escape() {
    let mut app = history_app();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.agents_modal = Some(crate::views::agents_modal::AgentsModalState::new(
        &agent.session.cwd,
        &std::collections::HashMap::new(),
        &crate::app::bundle::BundleState::default(),
        None, None, None,
    ));
    agent.line_viewer = crate::views::file_search::line_viewer::LineViewerState::open_markdown_content(
        "agent.md", "viewed definition".into(), None,
    );
    press(&mut app, KeyCode::Enter);
    assert_eq!(test_agent(&app, AgentId(0)).prompt.text(), "draft stays here");
    assert!(test_agent(&app, AgentId(0)).line_viewer.is_some());
    press(&mut app, KeyCode::Esc);
    let agent = test_agent(&app, AgentId(0));
    assert!(agent.line_viewer.is_none());
    assert!(agent.agents_modal.is_some());
    assert_eq!(agent.active_pane, ActivePane::Scrollback);
    press(&mut app, KeyCode::Esc);
    let agent = test_agent(&app, AgentId(0));
    assert!(agent.agents_modal.is_none());
    assert_eq!(agent.active_pane, ActivePane::Prompt);
    assert_eq!(agent.prompt.text(), "draft stays here");
}

#[test]
fn tab_skips_empty_dock_and_recovers_from_a_dock_that_became_empty() {
    for dock_shown in [false, true] {
        let mut app = history_app();
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.dock_shown = dock_shown;
        assert!(agent.dock_items().is_empty());
        press(&mut app, KeyCode::Tab);
        assert_eq!(test_agent(&app, AgentId(0)).active_pane, ActivePane::Prompt);
        app.agents.get_mut(&AgentId(0)).unwrap().active_pane = ActivePane::Dock;
        press(&mut app, KeyCode::Tab);
        assert_eq!(test_agent(&app, AgentId(0)).active_pane, ActivePane::Prompt);
    }
}

#[test]
fn tab_visits_nonempty_dock_at_either_edge_before_input() {
    for y in [0, 20] {
        let mut app = history_app();
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.subagent_sessions.insert("child".into(), make_test_subagent("child", "sa"));
        agent.dock_shown = true;
        agent.pane_areas.dock = ratatui::layout::Rect::new(0, y, 80, 4);
        assert!(!agent.dock_items().is_empty());
        press(&mut app, KeyCode::Tab);
        assert_eq!(test_agent(&app, AgentId(0)).active_pane, ActivePane::Dock);
        press(&mut app, KeyCode::Tab);
        assert_eq!(test_agent(&app, AgentId(0)).active_pane, ActivePane::Prompt);
    }
}

#[test]
fn tab_visits_visible_work_panes_once_before_input() {
    let mut app = history_app();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.pane_areas.todo = ratatui::layout::Rect::new(0, 0, 80, 3);
    agent.pane_areas.queue = ratatui::layout::Rect::new(0, 3, 80, 3);
    agent.pane_areas.tasks = ratatui::layout::Rect::new(0, 6, 80, 3);
    agent.todo.overlay.visible = true;
    agent.queue.overlay.visible = true;
    agent.tasks.overlay.visible = true;
    for expected in [ActivePane::Todo, ActivePane::Queue, ActivePane::Tasks, ActivePane::Prompt] {
        press(&mut app, KeyCode::Tab);
        assert_eq!(test_agent(&app, AgentId(0)).active_pane, expected);
    }
}

#[test]
fn tab_visits_open_sidebars_before_returning_to_input() {
    let mut app = history_app();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.adopt_global_sidebars(true, true, false, false);
    agent.note_terminal_size((160, 40));
    press(&mut app, KeyCode::Tab);
    assert!(test_agent(&app, AgentId(0)).f6_panel_focused());
    press(&mut app, KeyCode::Tab);
    assert!(test_agent(&app, AgentId(0)).f7_sidebar_focused());
    press(&mut app, KeyCode::Tab);
    let agent = test_agent(&app, AgentId(0));
    assert!(!agent.f6_panel_focused());
    assert!(!agent.f7_sidebar_focused());
    assert_eq!(agent.active_pane, ActivePane::Prompt);
}
