//! Full-pipeline mouse tests for the per-task action icons: `[↗]` (view) and
//! `[✗]` (Tasks pane kill) / `[stop]` (dock kill).
//!
//! Unlike `dock_input_tests`, nothing forges `pane_areas`: every test paints a
//! real frame with `draw`, locates the icon cells the frame actually painted,
//! and drives hover and click through `handle_mouse` at those exact cells. This
//! is the paint-vs-hit-test agreement the user exercises.
use super::test_fixtures::make_agent;
use super::{AgentView, BannerSlotParams};
use crate::actions::ActionRegistry;
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::scrollback::render::ScratchBuffer;
use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

#[path = "task_icon_scroll_tests.rs"]
mod scroll_tests;
/// The dock is hosted in the F7 sidebar: open it and let the shell "answer" so the frame paints.
fn open_dock_sidebar(agent: &mut AgentView) {
    let _ = agent.handle_input(
        &Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::F(7),
            KeyModifiers::NONE,
        )),
        &ActionRegistry::defaults(),
    );
}
fn reveal_f7_todo(agent: &mut AgentView) {
    agent.adopt_global_sidebars(false, true, false, true);
    for code in [crossterm::event::KeyCode::Home, crossterm::event::KeyCode::Down, crossterm::event::KeyCode::Down, crossterm::event::KeyCode::Down] {
        let _ = agent.handle_input(&Event::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE)), &ActionRegistry::defaults());
    }
}

fn cell_symbol(buf: &Buffer, x: u16, y: u16) -> String {
    buf.cell((x, y))
        .map(|c| c.symbol().to_string())
        .unwrap_or_else(|| panic!("missing cell at ({x},{y})"))
}
fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}
fn draw_frame(agent: &mut AgentView, area: Rect) -> Buffer {
    let mut buf = Buffer::empty(area);
    let mut scratch = ScratchBuffer::new();
    let _ = agent.draw(
        area,
        &mut buf,
        &ActionRegistry::defaults(),
        &mut scratch,
        None,
        false,
        BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        super::AppRenderParams::default(),
    );
    buf
}
/// All cells whose symbol matches `sym` exactly, in reading order.
fn find_symbol(buf: &Buffer, sym: &str) -> Vec<(u16, u16)> {
    let area = *buf.area();
    let mut hits = Vec::new();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if buf.cell((x, y)).is_some_and(|c| c.symbol() == sym) {
                hits.push((x, y));
            }
        }
    }
    hits
}
fn insert_running_task(agent: &mut AgentView, task_id: &str) {
    let entry_id = agent
        .scrollback
        .push_block(crate::scrollback::block::RenderBlock::BgTask(
            crate::scrollback::blocks::BgTaskBlock::started("sleep 5", task_id),
        ));
    insert_running_task_with_entry(agent, task_id, Some(entry_id));
}
fn insert_running_task_with_entry(
    agent: &mut AgentView,
    task_id: &str,
    scrollback_entry_id: Option<crate::scrollback::entry::EntryId>,
) {
    agent.session.bg_tasks.insert(
        task_id.into(),
        crate::app::agent::BgTaskState {
            task_id: task_id.into(),
            tool_call_id: format!("call-{task_id}"),
            command: "sleep 5".into(),
            description: None,
            cwd: "/tmp".into(),
            output_file: "/tmp/out".into(),
            status: crate::app::agent::BgTaskStatus::Running,
            start_time: std::time::SystemTime::now(),
            end_time: None,
            exit_code: None,
            signal: None,
            stdout: String::new(),
            stdout_line_count: 0,
            truncated: false,
            pending_kill: false,
            kill_requested_at: None,
            scrollback_entry_id,
            is_monitor: false,
            restored_from_replay: false,
        },
    );
}
#[test]
fn f7_task_icons_work_when_upper_dock_is_disabled() {
    check_work_icons(false);
}
/// A task whose scrollback entry never materialized (completed-early race) or
/// dangles after a scrollback swap must still open its viewer from `[↗]`: the
/// task state carries the stdout, and the dock path already opens it.
#[test]
fn f7_view_click_opens_viewer_without_scrollback_entry() {
    crate::views::dock::set_enabled_for_test(true);
    let mut agent = make_agent();
    open_dock_sidebar(&mut agent);
    insert_running_task_with_entry(&mut agent, "bg-1", None);
    let area = Rect::new(0, 0, 80, 30);
    let _ = draw_frame(&mut agent, area);
    let dock = agent.pane_areas.dock;
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, dock.x + 3, dock.y + 1));
    let buf = draw_frame(&mut agent, area);
    let view = *find_symbol(&buf, "\u{2197}")
        .first()
        .expect("view icon [↗] painted for a running task");
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, view.0, view.1));
    let outcome = agent.handle_mouse(&mouse(
        MouseEventKind::Down(MouseButton::Left),
        view.0,
        view.1,
    ));
    assert!(
        matches!(outcome, InputOutcome::Changed),
        "clicking the painted [↗] must open the viewer, got {outcome:?}"
    );
    assert!(
        agent.block_viewer.is_some(),
        "the [↗] click must open the bg task viewer even without a scrollback entry"
    );
    let buf = draw_frame(&mut agent, area);
    assert!(
        agent.block_viewer.is_some(),
        "the viewer must stay open across a repaint without a scrollback entry"
    );
    let painted: String = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| cell_symbol(&buf, x, y))
                .collect::<Vec<_>>()
                .concat()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        painted.contains("sleep 5"),
        "the viewer header must show the task command: {painted}"
    );
}
fn insert_running_subagent(agent: &mut AgentView, child_session_id: &str) {
    let info = super::test_fixtures::running_subagent_info(child_session_id);
    agent
        .subagent_sessions
        .insert(child_session_id.to_string(), info);
}
/// Dock subagent row: hovering reveals `[↗][stop]`; the painted `[stop]` must
/// kill the subagent and the painted `[↗]` must open its session view.
#[test]
fn dock_subagent_icons_hover_and_click_where_painted() {
    crate::views::dock::set_enabled_for_test(true);
    let mut agent = make_agent();
    open_dock_sidebar(&mut agent);
    insert_running_subagent(&mut agent, "child-1");
    let area = Rect::new(0, 0, 80, 30);
    let buf = draw_frame(&mut agent, area);
    assert!(agent.dock_on, "dock must be on for this test");
    let dock = agent.pane_areas.dock;
    assert!(dock.height >= 2, "dock painted: {dock:?}");
    let row_y = dock.y + 1;
    let row: String = (0..area.width)
        .map(|x| cell_symbol(&buf, x, row_y))
        .collect::<Vec<_>>()
        .concat();
    assert!(
        row.contains("Subagent test"),
        "subagent row painted: {row:?}"
    );
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, dock.x + 5, row_y));
    let buf = draw_frame(&mut agent, area);
    let row: String = (0..area.width)
        .map(|x| cell_symbol(&buf, x, row_y))
        .collect::<Vec<_>>()
        .concat();
    assert!(
        row.contains('\u{2197}') && row.contains("[stop]"),
        "hovered dock subagent row must show [↗][stop]: {row:?}"
    );
    let view_x = (0..area.width)
        .find(|x| cell_symbol(&buf, *x, row_y) == "\u{2197}")
        .expect("[↗] painted on hovered subagent row");
    let kill_x = (0..area.width)
        .rev()
        .find(|x| cell_symbol(&buf, *x, row_y) == "s")
        .expect("[stop] painted on hovered subagent row");
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, kill_x, row_y));
    let _ = draw_frame(&mut agent, area);
    let outcome = agent.handle_mouse(&mouse(
        MouseEventKind::Down(MouseButton::Left),
        kill_x,
        row_y,
    ));
    assert!(
        matches!(outcome, InputOutcome::Action(Action::KillSubagent(ref id)) if id == "sa-child-1"),
        "clicking the painted [stop] must kill the subagent, got {outcome:?}"
    );
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, view_x, row_y));
    let _ = draw_frame(&mut agent, area);
    let outcome = agent.handle_mouse(&mouse(
        MouseEventKind::Down(MouseButton::Left),
        view_x,
        row_y,
    ));
    assert!(
        matches!(outcome, InputOutcome::Action(Action::OpenSession(ref sid)) if sid == "child-1"),
        "clicking the painted [↗] must open the subagent's session, got {outcome:?}"
    );
}
/// Dock (remote `dock_enabled`): hovering a task row reveals `[↗][stop]`;
/// hovering and clicking the painted icons must act on that row.
#[test]
fn dock_icons_hover_and_click_where_painted() {
    check_work_icons(true);
}

fn check_work_icons(upper_enabled: bool) {
    crate::views::dock::set_enabled_for_test(true);
    let mut agent = make_agent();
    agent.tasks.overlay.visible = upper_enabled;
    open_dock_sidebar(&mut agent);
    insert_running_task(&mut agent, "bg-1");
    let area = Rect::new(0, 0, 80, 30);
    let buf = draw_frame(&mut agent, area);
    assert_eq!(agent.pane_areas.dock.x < agent.f7_body.x, upper_enabled);
    let dock = agent.pane_areas.dock;
    let row_y = (dock.y..dock.bottom())
        .find(|y| {
            let text: String = (0..area.width)
                .map(|x| cell_symbol(&buf, x, *y))
                .collect::<Vec<_>>()
                .concat();
            text.contains("sleep 5")
        })
        .expect("dock task row painted");
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, dock.x + 5, row_y));
    let buf = draw_frame(&mut agent, area);
    let row: String = (0..area.width)
        .map(|x| cell_symbol(&buf, x, row_y))
        .collect::<Vec<_>>()
        .concat();
    assert!(
        row.contains('\u{2197}') && row.contains("[stop]"),
        "hovered dock row must show [↗][stop]: {row:?}"
    );
    let view_x = (0..area.width)
        .find(|x| cell_symbol(&buf, *x, row_y) == "\u{2197}")
        .expect("[↗] cell");
    let stop_x = (0..area.width)
        .rev()
        .find(|x| cell_symbol(&buf, *x, row_y) == "s")
        .expect("[stop] cell");
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, stop_x, row_y));
    let _ = draw_frame(&mut agent, area);
    let outcome = agent.handle_mouse(&mouse(
        MouseEventKind::Down(MouseButton::Left),
        stop_x,
        row_y,
    ));
    assert!(
        matches!(outcome, InputOutcome::Action(Action::KillBgTask(ref id)) if id == "bg-1"),
        "clicking the painted [stop] must kill the task, got {outcome:?}"
    );
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, view_x, row_y));
    let _ = draw_frame(&mut agent, area);
    let outcome = agent.handle_mouse(&mouse(
        MouseEventKind::Down(MouseButton::Left),
        view_x,
        row_y,
    ));
    assert!(
        matches!(outcome, InputOutcome::Changed),
        "clicking the painted [↗] must open the task viewer, got {outcome:?}"
    );
    assert!(
        agent.block_viewer.is_some(),
        "the [↗] click must open the bg task viewer"
    );
}
fn dock_row_has_tasks(buf: &Buffer, dock: Rect) -> bool {
    if dock.height == 0 {
        return false;
    }
    (dock.y..dock.bottom()).any(|y| {
        let text: String = (0..buf.area().width)
            .map(|x| buf[(x, y)].symbol())
            .collect();
        text.contains("Tasks")
    })
}
#[test]
fn task_preference_only_changes_the_additional_upper_projection() {
    crate::views::dock::set_enabled_for_test(true);
    let mut agent = make_agent();
    open_dock_sidebar(&mut agent);
    insert_running_task(&mut agent, "bg-1");
    let area = Rect::new(0, 0, 160, 40);
    let registry = ActionRegistry::defaults();
    let ctrl_g = Event::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('g'),
        KeyModifiers::CONTROL,
    ));
    let buf = draw_frame(&mut agent, area);
    assert!(dock_row_has_tasks(&buf, agent.pane_areas.dock));
    assert!(agent.pane_areas.dock.x >= agent.f7_body.x);
    let _ = agent.handle_input(&ctrl_g, &registry);
    let buf = draw_frame(&mut agent, area);
    assert!(agent.tasks.overlay.visible);
    assert!(agent.pane_areas.dock.x < agent.f7_body.x);
    assert!(dock_row_has_tasks(&buf, agent.pane_areas.dock));
    let sidebar = agent.context_dock_geometry().expect("painted F7 dock").0;
    assert!(dock_row_has_tasks(&buf, sidebar));
    let _ = agent.handle_input(&ctrl_g, &registry);
    let buf = draw_frame(&mut agent, area);
    assert!(!agent.tasks.overlay.visible);
    assert!(dock_row_has_tasks(&buf, agent.pane_areas.dock));
}
#[test]
fn both_work_projections_open_the_same_child_with_surface_local_indices() {
    crate::views::dock::set_enabled_for_test(true);
    let mut agent = make_agent();
    agent.tasks.overlay.visible = true;
    agent.subagent_sessions.insert(
        "child-one".into(),
        super::test_fixtures::running_subagent_info("child-one"),
    );
    open_dock_sidebar(&mut agent);
    let registry = ActionRegistry::defaults();
    for size in [(160, 40), (120, 25)] {
        let area = Rect::new(0, 0, size.0, size.1);
        let _ = draw_frame(&mut agent, area);
        let upper = agent.pane_areas.dock;
        let sidebar = agent.context_dock_geometry().expect("painted F7 dock").0;
        assert!(upper.width > 0 && sidebar.width > 0);
        assert!(!upper.intersects(sidebar));
        for (rect, in_sidebar) in [(upper, false), (sidebar, true)] {
            let row = (rect.y..rect.bottom())
                .find(|y| {
                    matches!(
                        agent.dock_item_at(rect, *y),
                        Some(crate::views::dock::DockItem::Row(
                            crate::views::dock::Section::Subagents,
                            0
                        ))
                    )
                })
                .unwrap();
            let outcome = agent.handle_input(
                &Event::Mouse(mouse(
                    MouseEventKind::Down(MouseButton::Left),
                    rect.x + 3,
                    row,
                )),
                &registry,
            );
            assert!(
                matches!(outcome, InputOutcome::Action(Action::OpenSession(ref id)) if id == "child-one")
            );
            assert_eq!(agent.dock_focus_in_sidebar, in_sidebar);
            let enter = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                KeyModifiers::NONE,
            );
            assert!(
                matches!(agent.handle_dock_key(&enter), InputOutcome::Action(Action::OpenSession(ref id)) if id == "child-one")
            );
        }
    }
}

#[test]
fn suppressed_todo_releases_keyboard_focus_on_short_and_narrow_terminals() {
    crate::views::dock::set_enabled_for_test(true);
    let _theme = crate::theme::cache::pin_theme();
    let registry = ActionRegistry::defaults();
    for width in [80, 32, 24] {
        let mut agent = super::test_fixtures::running_agent_local_only();
        agent.vim_mode = false;
        agent
            .todo
            .update_todos(vec![xai_grok_shell::tools::TodoItem {
                content: "resize-todo".into(),
                priority: Default::default(),
                status: xai_grok_shell::tools::TodoStatus::Pending,
                meta: None,
            }]);
        let large = Rect::new(0, 0, 80, 30);
        let _ = draw_frame(&mut agent, large);
        let _ = agent.handle_input(
            &Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('t'),
                KeyModifiers::CONTROL,
            )),
            &registry,
        );
        let _ = draw_frame(&mut agent, large);
        assert_eq!(agent.active_pane, super::AgentPane::Todo);
        assert!(agent.pane_areas.todo.height > 0);
        let small = Rect::new(0, 0, width, 16);
        let _ = agent.handle_input(&Event::Resize(width, 16), &registry);
        let _ = draw_frame(&mut agent, small);
        assert_eq!(agent.pane_areas.todo.height, 0);
        assert_eq!(agent.active_pane, super::AgentPane::Scrollback);
        assert!(!agent.todo.overlay.focused);
        assert!(agent.todo.is_visible());
        assert!(agent.pane_areas.queue.height > 0);
        assert_eq!(agent.pane_areas.queue.bottom(), agent.pane_areas.prompt.y);
        let show_done = agent.todo.show_done();
        let outcome = agent.handle_input(
            &Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('h'),
                KeyModifiers::NONE,
            )),
            &registry,
        );
        assert!(matches!(
            outcome,
            InputOutcome::ActionThenForward(Action::FocusPrompt)
        ));
        assert_eq!(agent.todo.show_done(), show_done);
        let _ = draw_frame(&mut agent, large);
        assert!(agent.pane_areas.todo.height > 0);
        assert_eq!(agent.active_pane, super::AgentPane::Scrollback);
    }
}

#[test]
fn upper_todo_render_obeys_visibility_in_every_dock_placement_without_hiding_f7_or_queue() {
    let _theme = crate::theme::cache::pin_theme();
    let registry = ActionRegistry::defaults();
    let toggle_todo = Event::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('t'),
        KeyModifiers::CONTROL,
    ));
    for placement in [crate::views::dock::DockPlacement::Off, crate::views::dock::DockPlacement::Top, crate::views::dock::DockPlacement::Bottom] {
        crate::views::dock::set_enabled_for_test(false);
        crate::views::dock::set_placement(placement);
        let dock = placement.is_visible();
        for f7 in [false, true] {
            for tasks in [false, true] {
                for empty in [false, true] {
                    let mut agent = super::test_fixtures::running_agent_local_only();
                    agent.tasks.overlay.visible = tasks;
                    agent.todo.overlay.visible = false;
                    insert_running_task(&mut agent, "bg-todo-visibility");
                    if !empty {
                        agent
                            .todo
                            .update_todos(vec![xai_grok_shell::tools::TodoItem {
                                content: "upper-todo-marker".into(),
                                priority: Default::default(),
                                status: xai_grok_shell::tools::TodoStatus::Pending,
                                meta: None,
                            }]);
                    }
                    if f7 {
                        open_dock_sidebar(&mut agent);
                    }
                    for visible in [false, true, false] {
                        if agent.todo.is_visible() != visible {
                            let _ = agent.handle_input(&toggle_todo, &registry);
                        }
                        for height in [40, 24] {
                            let area = Rect::new(0, 0, 160, height);
                            let buf = draw_frame(&mut agent, area);
                            let todo = agent.pane_areas.todo;
                            assert_eq!(todo.height > 0, visible);
                            let upper_width = if f7 { agent.f7_body.x } else { area.width };
                            let upper: String = (0..height)
                                .flat_map(|y| (0..upper_width).map(move |x| (x, y)))
                                .map(|(x, y)| cell_symbol(&buf, x, y))
                                .collect();
                            let marker = if empty {
                                "No todo items."
                            } else {
                                "upper-todo-marker"
                            };
                            assert_eq!(upper.contains(marker), visible);
                            assert!(agent.pane_areas.queue.height > 0);
                            assert_eq!(agent.pane_areas.queue.bottom(), agent.pane_areas.prompt.y);
                            assert_eq!(agent.dock_counts().queued, 0);
                            assert!(agent.hit_queue_close.rect.is_none());
                            if f7 {
                                if dock {
                                    let sidebar = agent.context_dock_geometry().expect("painted F7 dock").0;
                                    assert!(dock_row_has_tasks(&buf, sidebar));
                                } else {
                                    assert!(agent.context_dock_geometry().is_none());
                                }
                                if !empty {
                                    reveal_f7_todo(&mut agent);
                                    let buf = draw_frame(&mut agent, area);
                                    let sidebar = agent.f7_body;
                                    let text: String = (sidebar.y..sidebar.bottom())
                                        .flat_map(|y| {
                                            (sidebar.x..sidebar.right()).map(move |x| (x, y))
                                        })
                                        .map(|(x, y)| cell_symbol(&buf, x, y))
                                        .collect();
                                    assert!(text.contains("upper-todo-marker"));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn f7_keeps_work_and_todos_when_upper_dock_and_preferences_are_off() {
    for placement in [crate::views::dock::DockPlacement::Off, crate::views::dock::DockPlacement::Top, crate::views::dock::DockPlacement::Bottom] {
        crate::views::dock::set_enabled_for_test(false);
        crate::views::dock::set_placement(placement);
        let dock = placement.is_visible();
        for visible in [false, true] {
            let mut agent = make_agent();
            insert_running_task(&mut agent, "bg-1");
            agent.tasks.overlay.visible = visible;
            agent.todo.overlay.visible = visible;
            agent
                .todo
                .update_todos(vec![xai_grok_shell::tools::TodoItem {
                    content: "persistent-todo-marker".into(),
                    priority: Default::default(),
                    status: xai_grok_shell::tools::TodoStatus::Pending,
                    meta: None,
                }]);
            let mut child = super::test_fixtures::running_subagent_info("manual-child");
            child.attempt.delivery =
                xai_grok_shell::extensions::notification::SubagentDelivery::Held;
            child.attempt.completion =
                xai_grok_shell::extensions::notification::SubagentCompletion::Pending;
            child.attempt.caller = xai_grok_tools::implementations::grok_build::task::coordinator::SubagentCaller::Waiting;
            agent.subagent_sessions.insert("manual-child".into(), child);
            open_dock_sidebar(&mut agent);
            let area = Rect::new(0, 0, 160, 40);
            let buf = draw_frame(&mut agent, area);
            if dock {
                let sidebar = agent.context_dock_geometry().expect("painted F7 dock").0;
                assert!(dock_row_has_tasks(&buf, sidebar));
                let warning: String = (sidebar.y..sidebar.bottom())
                    .flat_map(|y| (sidebar.x..sidebar.right()).map(move |x| (x, y)))
                    .map(|(x, y)| cell_symbol(&buf, x, y))
                    .collect();
                assert!(warning.contains("MANUAL"));
                assert!(warning.contains("CALLER BLOCKED"));
            } else {
                assert!(agent.context_dock_geometry().is_none());
                let sidebar = agent.f7_body;
                let text: String = (sidebar.y..sidebar.bottom())
                    .flat_map(|y| (sidebar.x..sidebar.right()).map(move |x| (x, y)))
                    .map(|(x, y)| cell_symbol(&buf, x, y)).collect();
                assert!(text.contains("Dock") && text.contains("Off"));
            }
            reveal_f7_todo(&mut agent);
            let buf = draw_frame(&mut agent, area);
            let text: String = (agent.f7_body.y..agent.f7_body.bottom())
                .flat_map(|y| (agent.f7_body.x..agent.f7_body.right()).map(move |x| (x, y)))
                .map(|(x, y)| cell_symbol(&buf, x, y))
                .collect();
            assert!(
                text.contains("persistent-todo-marker"),
                "F7 keyboard navigation must reach real Todos independently of upper display preferences"
            );
            assert_eq!(agent.todo.todos()[0].content, "persistent-todo-marker");
            assert_eq!(agent.session.bg_tasks.len(), 1);
        }
    }
}
