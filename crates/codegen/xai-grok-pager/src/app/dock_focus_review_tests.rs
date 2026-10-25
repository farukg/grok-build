use super::*;
use crate::app::actions::Action;
use crate::app::agent::AgentId;
use crate::app::agent_view::{AgentPane, VisibleFocus};
use crate::views::btw_overlay::BtwOverlayState;
use crate::views::dock::{self, DockItem, DockPlacement, Section};
use ratatui::{buffer::Buffer, layout::Rect};

fn press(app: &mut AppView, code: KeyCode, modifiers: KeyModifiers) {
    if let InputOutcome::Action(action) = app.handle_input(&Event::Key(KeyEvent::new(code, modifiers))) {
        let _ = crate::app::dispatch::dispatch(action, app);
    }
}

fn setup(placement: DockPlacement) -> (AppView, tempfile::TempDir) {
    dock::set_enabled_for_test(false);
    let settings = tempfile::tempdir().unwrap();
    let mut app = test_app_with_agent();
    app.screen_mode = ScreenMode::Fullscreen;
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.open_panel_settings_in(settings.path());
    assert!(agent.set_dock_placement(placement));
    agent.tasks.overlay.visible = false;
    agent.todo.overlay.visible = false;
    agent.adopt_global_sidebars(false, false, false, false);
    agent.restore_visible_focus(VisibleFocus::Pane(AgentPane::Prompt));
    app.sidebar_chrome.work_visibility = Some((false, false));
    (app, settings)
}

fn populate_work(app: &mut AppView) {
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let mut task = crate::app::agent_test_fixtures::running_bg_task("r3-task", false);
    task.command = "r3-task-command".into();
    agent.session.bg_tasks.insert("r3-task".into(), task);
    let mut child = crate::app::agent_view::test_fixtures::running_subagent_info("r3-child");
    child.description = std::sync::Arc::from("r3-child-description");
    agent.subagent_sessions.insert("r3-child".into(), child);
    agent.workflow_runs.push(crate::views::workflows::WorkflowRunSnapshot {
        run_id: "r3-run".into(), name: "r3-workflow".into(), objective: "objective".into(),
        status: "active".into(), management_available: true, builtin: false,
        phases: Vec::new(), current_phase: None, agents: Vec::new(), agent_budget: None,
        agents_used: 0, agents_reserved: 0, agents_remaining: None, agent_usage_incomplete: false,
        active_agents: 0, elapsed_ms: 0, received_at: std::time::Instant::now(),
        pause_message: None, result_summary: None,
    });
}

fn row_text(buf: &Buffer, rect: Rect, y: u16) -> String {
    (rect.x..rect.right()).map(|x| buf.cell((x, y)).unwrap().symbol()).collect()
}

fn painted_row(buf: &Buffer, rect: Rect, label: &str) -> u16 {
    let rows: Vec<_> = (rect.y..rect.bottom()).filter(|&y| row_text(buf, rect, y).contains(label)).collect();
    assert_eq!(rows.len(), 1, "exactly one painted row for {label} in {rect:?}");
    rows[0]
}

fn mouse_input(app: &mut AppView, kind: MouseEventKind, x: u16, y: u16) -> InputOutcome {
    app.agents.get_mut(&AgentId(0)).unwrap().handle_input(
        &Event::Mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }),
        &crate::actions::ActionRegistry::defaults(),
    )
}

#[test]
fn appview_f7_work_rows_keep_exact_actions_with_standalone_off_top_or_bottom() {
    let _theme = crate::theme::cache::pin_theme();
    for placement in [DockPlacement::Off, DockPlacement::Top, DockPlacement::Bottom] {
        for section in [Section::Workflows, Section::Subagents, Section::Tasks] {
            let (mut app, _settings) = setup(placement);
            populate_work(&mut app);
            press(&mut app, KeyCode::F(7), KeyModifiers::NONE);
            let (mut terminal, _frames) = crate::test_util::test_terminal_in(Rect::new(0, 0, 220, 50));
            app.draw(&mut terminal);
            let agent = app.agents.get(&AgentId(0)).unwrap();
            assert!(!agent.tasks.overlay.visible);
            let rect = agent.context_dock_geometry().expect("F7 paints independently of placement").0;
            assert_eq!(agent.pane_areas.dock, rect, "only the hosted projection paints");
            let buf = terminal.completed_buffer();
            let workflow_y = painted_row(buf, rect, "r3-workflow");
            let child_y = painted_row(buf, rect, "r3-child-description");
            let task_y = painted_row(buf, rect, "r3-task-command");
            assert!(workflow_y < child_y && child_y < task_y);
            assert_eq!(agent.dock_item_at(rect, workflow_y), Some(DockItem::Row(Section::Workflows, 0)));
            assert_eq!(agent.dock_item_at(rect, child_y), Some(DockItem::Row(Section::Subagents, 0)));
            assert_eq!(agent.dock_item_at(rect, task_y), Some(DockItem::Row(Section::Tasks, 0)));
            let y = match section {
                Section::Workflows => workflow_y,
                Section::Subagents => child_y,
                Section::Tasks => task_y,
                Section::Watchers | Section::Queued => unreachable!(),
            };
            let _ = mouse_input(&mut app, MouseEventKind::Moved, rect.x + 3, y);
            app.draw(&mut terminal);
            let buf = terminal.completed_buffer();
            let view_x = (rect.x..rect.right()).find(|&x| buf.cell((x, y)).unwrap().symbol() == "↗").unwrap();
            let stop_x = (rect.x..rect.right().saturating_sub(5)).find(|&x| {
                ["[", "s", "t", "o", "p", "]"].iter().enumerate()
                    .all(|(offset, symbol)| buf.cell((x + offset as u16, y)).unwrap().symbol() == *symbol)
            }).expect("the actual hovered row paints its stop action") + 1;
            let _ = mouse_input(&mut app, MouseEventKind::Moved, stop_x, y);
            app.draw(&mut terminal);
            let stop = mouse_input(&mut app, MouseEventKind::Down(MouseButton::Left), stop_x, y);
            match section {
                Section::Workflows => assert!(matches!(stop, InputOutcome::Action(Action::SendSlashCommandPreservingDraft(ref command)) if command == "/workflow stop r3-workflow")),
                Section::Subagents => assert!(matches!(stop, InputOutcome::Action(Action::KillSubagent(ref id)) if id == "sa-r3-child")),
                Section::Tasks => assert!(matches!(stop, InputOutcome::Action(Action::KillBgTask(ref id)) if id == "r3-task")),
                Section::Watchers | Section::Queued => unreachable!(),
            }
            let _ = mouse_input(&mut app, MouseEventKind::Moved, view_x, y);
            app.draw(&mut terminal);
            let open = mouse_input(&mut app, MouseEventKind::Down(MouseButton::Left), view_x, y);
            let agent = app.agents.get(&AgentId(0)).unwrap();
            match section {
                Section::Workflows => {
                    assert!(matches!(open, InputOutcome::Changed));
                    assert_eq!(agent.workflows_view.detail_run_id.as_deref(), Some("r3-run"));
                }
                Section::Subagents => assert!(matches!(open, InputOutcome::Action(Action::OpenSession(ref id)) if id == "r3-child")),
                Section::Tasks => {
                    assert!(matches!(open, InputOutcome::Changed));
                    assert_eq!(agent.block_viewer.as_ref().unwrap().bg_task_id.as_deref(), Some("r3-task"));
                }
                Section::Watchers | Section::Queued => unreachable!(),
            }
        }
    }
}

#[test]
fn appview_tab_visits_each_painted_dock_projection_once_and_obeys_f7_disclosure() {
    let _theme = crate::theme::cache::pin_theme();
    for placement in [DockPlacement::Off, DockPlacement::Top, DockPlacement::Bottom] {
        let (mut app, _settings) = setup(placement);
        populate_work(&mut app);
        press(&mut app, KeyCode::F(7), KeyModifiers::NONE);
        let (mut terminal, _frames) = crate::test_util::test_terminal_in(Rect::new(0, 0, 220, 50));
        app.draw(&mut terminal);
        app.agents.get_mut(&AgentId(0)).unwrap().restore_visible_focus(VisibleFocus::Context);
        press(&mut app, KeyCode::Home, KeyModifiers::NONE);
        press(&mut app, KeyCode::Left, KeyModifiers::NONE);
        app.draw(&mut terminal);
        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert!(agent.context_dock_geometry().is_none());
        assert_eq!(agent.pane_areas.dock.area(), 0);
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(app.agents.get(&AgentId(0)).unwrap().visible_focus(), VisibleFocus::Pane(AgentPane::Prompt));
        app.agents.get_mut(&AgentId(0)).unwrap().restore_visible_focus(VisibleFocus::Context);
        press(&mut app, KeyCode::Home, KeyModifiers::NONE);
        press(&mut app, KeyCode::Right, KeyModifiers::NONE);
        app.draw(&mut terminal);
        let rect = app.agents.get(&AgentId(0)).unwrap().context_dock_geometry().unwrap().0;
        for label in ["r3-workflow", "r3-child-description", "r3-task-command"] {
            painted_row(terminal.completed_buffer(), rect, label);
        }
        for upper in [false, true] {
            if upper { press(&mut app, KeyCode::Char('g'), KeyModifiers::CONTROL); }
            app.draw(&mut terminal);
            app.agents.get_mut(&AgentId(0)).unwrap().restore_visible_focus(VisibleFocus::Pane(AgentPane::Prompt));
            let mut expected = vec![VisibleFocus::Pane(AgentPane::Scrollback), VisibleFocus::Context, VisibleFocus::SidebarDock];
            if upper {
                expected.push(VisibleFocus::Pane(match placement {
                    DockPlacement::Off => AgentPane::Tasks,
                    DockPlacement::Top | DockPlacement::Bottom => AgentPane::Dock,
                }));
            }
            expected.push(VisibleFocus::Pane(AgentPane::Prompt));
            for focus in expected {
                press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
                app.draw(&mut terminal);
                assert_eq!(app.agents.get(&AgentId(0)).unwrap().visible_focus(), focus, "{placement:?}, CtrlG {upper}");
            }
        }
        app.agents.get_mut(&AgentId(0)).unwrap().restore_visible_focus(VisibleFocus::Context);
        press(&mut app, KeyCode::Home, KeyModifiers::NONE);
        press(&mut app, KeyCode::Left, KeyModifiers::NONE);
        app.draw(&mut terminal);
        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert!(agent.context_dock_geometry().is_none());
        assert!(agent.tasks.overlay.visible);
        let f7 = agent.f7_body;
        for y in f7.y..f7.bottom() {
            let text = row_text(terminal.completed_buffer(), f7, y);
            assert!(!text.contains("r3-workflow") && !text.contains("r3-child-description") && !text.contains("r3-task-command"));
        }
        press(&mut app, KeyCode::Right, KeyModifiers::NONE);
        app.draw(&mut terminal);
        assert!(app.agents.get(&AgentId(0)).unwrap().context_dock_geometry().is_some());
        app.agents.get_mut(&AgentId(0)).unwrap().restore_visible_focus(VisibleFocus::SidebarDock);
        app.agents.get_mut(&AgentId(0)).unwrap().dock_cursor = usize::MAX;
        app.draw(&mut terminal);
        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert!(agent.dock_cursor < agent.dock_items().len());
    }
}

#[test]
fn appview_short_terminal_suppresses_only_the_standalone_dock() {
    let _theme = crate::theme::cache::pin_theme();
    for placement in [DockPlacement::Top, DockPlacement::Bottom] {
        let (mut app, _settings) = setup(placement);
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        let mut task = crate::app::agent_test_fixtures::running_bg_task("short-task", false);
        task.command = "short-task-command".into();
        agent.session.bg_tasks.insert("short-task".into(), task);
        press(&mut app, KeyCode::Char('g'), KeyModifiers::CONTROL);
        press(&mut app, KeyCode::F(7), KeyModifiers::NONE);
        let (mut terminal, _frames) = crate::test_util::test_terminal_in(Rect::new(0, 0, 220, 16));
        app.draw(&mut terminal);
        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert!(!agent.dock_on);
        let (hosted, _) = agent.context_dock_geometry().expect("short height retains paintable F7 work");
        assert_eq!(agent.pane_areas.dock, hosted);
        painted_row(terminal.completed_buffer(), hosted, "short-task-command");
    }
}

#[test]
fn appview_btw_focus_preserves_prompt_completion_priority() {
    let _theme = crate::theme::cache::pin_theme();
    let (mut app, _settings) = setup(DockPlacement::Off);
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.btw_state = Some(BtwOverlayState::done("question".into(), "answer".into()));
    agent.prompt.set_text("/hel");
    agent.prompt.refresh_slash(&agent.session.models);
    assert!(agent.prompt.slash_open());
    agent.restore_visible_focus(VisibleFocus::Btw);
    let (mut terminal, _frames) = crate::test_util::test_terminal_in(Rect::new(0, 0, 160, 40));
    app.draw(&mut terminal);
    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    let agent = app.agents.get(&AgentId(0)).unwrap();
    assert_eq!(agent.prompt.text().trim(), "/help");
    assert_eq!(agent.visible_focus(), VisibleFocus::Pane(AgentPane::Prompt));
    assert!(!agent.btw_focused);
}

#[test]
fn appview_tab_reaches_painted_btw_scrolls_only_its_body_and_restores_session_owner() {
    let _theme = crate::theme::cache::pin_theme();
    let (mut app, _settings) = setup(DockPlacement::Off);
    let root = AgentId(0);
    let answer = (0..80).map(|n| format!("btw answer line {n:02}  \n")).collect::<String>();
    let agent = app.agents.get_mut(&root).unwrap();
    agent.btw_state = Some(BtwOverlayState::done("btw question".into(), answer.clone()));
    agent.prompt.set_text("draft remains");
    agent.scrollback.push_block(crate::scrollback::block::RenderBlock::agent_message(
        (0..60).map(|n| format!("history line {n:02}  \n")).collect::<String>(),
    ));
    agent.scrollback.set_selected(Some(0));
    let (mut terminal, _frames) = crate::test_util::test_terminal_in(Rect::new(0, 0, 160, 40));
    app.draw(&mut terminal);
    let agent = app.agents.get(&root).unwrap();
    painted_row(terminal.completed_buffer(), agent.last_btw_area, "btw answer line 00");
    let prompt_cursor = agent.prompt.cursor();
    let selected = agent.scrollback.selected();
    for focus in [VisibleFocus::Pane(AgentPane::Scrollback), VisibleFocus::Btw] {
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        app.draw(&mut terminal);
        assert_eq!(app.agents.get(&root).unwrap().visible_focus(), focus);
    }
    let before_scroll = terminal.completed_buffer().clone();
    press(&mut app, KeyCode::Down, KeyModifiers::NONE);
    app.draw(&mut terminal);
    let agent = app.agents.get(&root).unwrap();
    assert_eq!(agent.btw_state.as_ref().unwrap().scroll_offset(), 1);
    painted_row(terminal.completed_buffer(), agent.last_btw_area, "btw answer line 01");
    assert_eq!(agent.prompt.text(), "draft remains");
    assert_eq!(agent.prompt.cursor(), prompt_cursor);
    assert_eq!(agent.scrollback.selected(), selected);
    for rect in [agent.pane_areas.scrollback, agent.pane_areas.prompt] {
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                assert_eq!(terminal.completed_buffer().cell((x, y)), before_scroll.cell((x, y)));
            }
        }
    }
    let second = AgentId(1);
    let mut other = idle_child_view(&app, 1, "btw-second");
    other.btw_state = Some(BtwOverlayState::done("second question".into(), answer));
    app.agents.insert(second, *other);
    for (session, id) in [("btw-second", second), ("test-session", root)] {
        app.capture_sidebar_chrome();
        let _ = crate::app::dispatch::dispatch(Action::OpenSession(session.into()), &mut app);
        app.draw(&mut terminal);
        assert_eq!(app.active_view, ActiveView::Agent(id));
        assert_eq!(app.agents.get(&id).unwrap().visible_focus(), VisibleFocus::Btw);
    }
    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.agents.get(&root).unwrap().visible_focus(), VisibleFocus::Pane(AgentPane::Prompt));
    assert!(!app.agents.get(&root).unwrap().btw_focused);
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    app.draw(&mut terminal);
    assert!(app.agents.get(&root).unwrap().btw_state.is_none());
    for focus in [VisibleFocus::Pane(AgentPane::Scrollback), VisibleFocus::Pane(AgentPane::Prompt)] {
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(app.agents.get(&root).unwrap().visible_focus(), focus);
    }
}
