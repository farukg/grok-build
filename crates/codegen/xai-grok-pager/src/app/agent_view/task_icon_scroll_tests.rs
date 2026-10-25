use super::*;
use crate::views::dock::{self, DockItem, DockPlacement, Section};

fn stop_cell(buf: &Buffer, area: Rect, row: u16) -> (u16, u16) {
    let x = (area.x..area.right().saturating_sub(5)).find(|&x| {
        ["[", "s", "t", "o", "p", "]"].iter().enumerate()
            .all(|(offset, symbol)| cell_symbol(buf, x + offset as u16, row) == *symbol)
    }).expect("a visible stop icon must be painted on the hovered row");
    (x + 1, row)
}

fn is_stop(outcome: &InputOutcome) -> bool {
    matches!(outcome, InputOutcome::Action(Action::KillBgTask(_) | Action::KillSubagent(_)
        | Action::SendSlashCommandPreservingDraft(_)))
}

#[test]
fn dock_placements_remain_disjoint_through_search_timeline_and_width_recomputation() {
    use crossterm::event::{KeyCode, KeyEvent};
    let _theme = crate::theme::cache::pin_theme();
    dock::set_enabled_for_test(false);
    for placement in [DockPlacement::Off, DockPlacement::Top, DockPlacement::Bottom] {
        dock::set_placement(placement);
        let mut agent = make_agent();
        agent.vim_mode = true;
        agent.tasks.overlay.visible = true;
        insert_running_task(&mut agent, "layout-task");
        insert_running_subagent(&mut agent, "layout-child");
        agent.workflow_runs.push(super::super::dock_input_tests::workflow_run("layout-workflow", "layout-workflow-name", "active"));
        agent.todo.update_todos(vec![xai_grok_shell::tools::TodoItem {
            content: "layout-todo".into(), priority: Default::default(),
            status: xai_grok_shell::tools::TodoStatus::Pending, meta: None,
        }]);
        agent.todo.overlay.visible = true;
        open_dock_sidebar(&mut agent);
        let registry = ActionRegistry::defaults();
        let _ = agent.handle_input(&Event::Key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)), &registry);
        agent.restore_visible_focus(super::super::VisibleFocus::Pane(super::super::AgentPane::Scrollback));
        let _ = agent.handle_input(&Event::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)), &registry);
        assert!(agent.scrollback_search.is_some());
        for width in [220, 160, 110, 220] {
            let area = Rect::new(0, 0, width, 40);
            let buf = draw_frame(&mut agent, area);
            assert!(agent.pane_areas.todo.height > 0);
            let dock_rect = agent.pane_areas.dock;
            match placement {
                DockPlacement::Off => {
                    assert!(agent.pane_areas.tasks.height > 0);
                    if let Some((hosted, _)) = agent.context_dock_geometry() {
                        assert_eq!(dock_rect, hosted);
                        assert!(!hosted.intersects(agent.pane_areas.tasks));
                    } else {
                        assert_eq!(dock_rect.height, 0);
                    }
                }
                DockPlacement::Top => assert!(dock_rect.bottom() <= agent.pane_areas.scrollback.y),
                DockPlacement::Bottom => assert!(dock_rect.y >= agent.pane_areas.scrollback.bottom()),
            }
            for neighbor in [agent.pane_areas.scrollback, agent.pane_areas.todo, agent.pane_areas.queue, agent.pane_areas.prompt] {
                assert!(!dock_rect.intersects(neighbor));
            }
            let f7 = agent.f7_body;
            let text: String = (f7.y..f7.bottom()).flat_map(|y| (f7.x..f7.right()).map(move |x| (x, y)))
                .map(|(x, y)| cell_symbol(&buf, x, y)).collect();
            assert!(text.contains("Dock") && text.contains("Todos"));
            if let Some((sidebar, _)) = agent.context_dock_geometry() {
                match placement {
                    DockPlacement::Off => assert_eq!(sidebar, dock_rect),
                    DockPlacement::Top | DockPlacement::Bottom => assert!(!sidebar.intersects(dock_rect)),
                }
            }
        }
    }
}

#[test]
fn scrolled_f7_stop_icons_target_only_the_painted_task_subagent_or_workflow() {
    let _theme = crate::theme::cache::pin_theme();
    dock::set_enabled_for_test(true);
    for placement in [DockPlacement::Top, DockPlacement::Bottom] {
        dock::set_placement(placement);
        for section in [Section::Workflows, Section::Subagents, Section::Tasks] {
            let mut agent = make_agent();
            insert_running_task(&mut agent, "visible-task");
            insert_running_subagent(&mut agent, "visible-child");
            agent.workflow_runs.push(super::super::dock_input_tests::workflow_run("visible-workflow", "visible-workflow-name", "active"));
            open_dock_sidebar(&mut agent);
            let area = Rect::new(0, 0, 160, 40);
            let _ = draw_frame(&mut agent, area);
            let target = DockItem::Row(section, 0);
            let initial = agent.context_dock_geometry().unwrap().0;
            let initial_row = (initial.y..initial.bottom()).find(|&row| agent.dock_item_at(initial, row) == Some(target)).unwrap();
            let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, initial.x + 3, initial_row));
            let initial_buf = draw_frame(&mut agent, area);
            let initial_stop = stop_cell(&initial_buf, initial, initial_row);
            let mut reached = false;
            for _ in 0..12 {
                let body = agent.f7_body;
                let _ = agent.handle_input(&Event::Mouse(mouse(MouseEventKind::ScrollDown, body.x + 3, body.y + 3)), &ActionRegistry::defaults());
                let _ = draw_frame(&mut agent, area);
                if let Some((rect, offset)) = agent.context_dock_geometry()
                    && offset > 0 && agent.dock_item_at(rect, rect.y) == Some(target)
                {
                    reached = true;
                    break;
                }
            }
            assert!(reached, "the actual F7 viewport must clip rows above the target");
            let (rect, offset) = agent.context_dock_geometry().unwrap();
            assert!(offset > 0);
            let row = rect.y;
            assert_ne!(row, initial_stop.1);
            let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, rect.x + 3, row));
            let buf = draw_frame(&mut agent, area);
            let (x, y) = stop_cell(&buf, rect, row);
            let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, x, y));
            let _ = draw_frame(&mut agent, area);
            let outcome = agent.handle_mouse(&mouse(MouseEventKind::Down(MouseButton::Left), x, y));
            match section {
                Section::Workflows => assert!(matches!(outcome, InputOutcome::Action(Action::SendSlashCommandPreservingDraft(ref command)) if command == "/workflow stop visible-workflow-name")),
                Section::Subagents => assert!(matches!(outcome, InputOutcome::Action(Action::KillSubagent(ref id)) if id == "sa-visible-child")),
                Section::Tasks => assert!(matches!(outcome, InputOutcome::Action(Action::KillBgTask(ref id)) if id == "visible-task")),
                Section::Watchers | Section::Queued => unreachable!("test cases are explicitly enumerated"),
            }
            if section == Section::Subagents {
                let view_x = (rect.x..rect.right()).find(|&column| cell_symbol(&buf, column, row) == "\u{2197}")
                    .expect("the shifted row must retain its painted view action");
                let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, view_x, row));
                let _ = draw_frame(&mut agent, area);
                let outcome = agent.handle_mouse(&mouse(MouseEventKind::Down(MouseButton::Left), view_x, row));
                assert!(matches!(outcome, InputOutcome::Action(Action::OpenSession(ref id)) if id == "visible-child"));
            }
            let neighbor = agent.handle_mouse(&mouse(MouseEventKind::Down(MouseButton::Left), x, rect.bottom()));
            assert!(!is_stop(&neighbor), "the Todo section immediately below the clipped dock must not inherit its stop hit");
            let above = agent.handle_mouse(&mouse(MouseEventKind::Down(MouseButton::Left), x, rect.y.saturating_sub(1)));
            assert!(!is_stop(&above), "clipped rows above the viewport cannot remain clickable");
            assert_eq!(agent.session.bg_tasks.len(), 1);
            assert_eq!(agent.subagent_sessions.len(), 1);
            assert_eq!(agent.workflow_runs.len(), 1);
        }
    }
}
