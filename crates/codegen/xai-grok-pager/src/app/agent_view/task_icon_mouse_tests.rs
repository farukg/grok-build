//! Full-pipeline mouse tests for the per-task action icons: `[↗]` (view) and
//! `[✗]` (Tasks pane kill) / `[stop]` (dock kill).
//!
//! Unlike `dock_input_tests`, nothing forges `pane_areas`: every test paints a
//! real frame with `draw`, locates the icon cells the frame actually painted,
//! and drives hover and click through `handle_mouse` at those exact cells. This
//! is the paint-vs-hit-test agreement the user exercises.
use super::test_fixtures::make_agent;
use crate::app::agent::AgentId;
use crate::app::app_view::{ActiveView, AppView};
use crate::app::session_views::SessionViews;
use super::{AgentView, BannerSlotParams};
use crate::actions::ActionRegistry;
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::scrollback::render::ScratchBuffer;
use crate::views::tasks_pane::TaskEntryId;
use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
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
/// Tasks pane (dock off): the frame paints `[↗]` and `[✗]` on a running task
/// row; hovering each icon activates it and clicking fires its action.
#[test]
fn tasks_pane_icons_hover_and_click_where_painted() {
    let mut agent = make_agent();
    insert_running_task(&mut agent, "bg-1");
    let area = Rect::new(0, 0, 80, 30);
    let _ = draw_frame(agent, area);
    let buf = draw_frame(&mut agent, area);
    let kill = *find_symbol(&buf, "\u{2717}")
        .first()
        .expect("kill icon [✗] painted for a running task");
    let view = *find_symbol(&buf, "\u{2197}")
        .first()
        .expect("view icon [↗] painted for a running task");
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, kill.0, kill.1));
    assert_eq!(
        agent.tasks.hovered_kill,
        Some(TaskEntryId::BgTask("bg-1".into())),
        "hovering the painted [✗] must activate it"
    );
    let outcome = agent.handle_mouse(&mouse(
        MouseEventKind::Down(MouseButton::Left),
        kill.0,
        kill.1,
    ));
    assert!(
        matches!(outcome, InputOutcome::Action(Action::KillBgTask(ref id)) if id == "bg-1"),
        "clicking the painted [✗] must kill the task, got {outcome:?}"
    );
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, view.0, view.1));
    assert_eq!(
        agent.tasks.hovered_view,
        Some(TaskEntryId::BgTask("bg-1".into())),
        "hovering the painted [↗] must activate it"
    );
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
        "the [↗] click must open the bg task viewer"
    );
}
/// A task whose scrollback entry never materialized (completed-early race) or
/// dangles after a scrollback swap must still open its viewer from `[↗]`: the
/// task state carries the stdout, and the dock path already opens it.
#[test]
fn tasks_pane_view_click_opens_viewer_without_scrollback_entry() {
    let mut agent = make_agent();
    insert_running_task_with_entry(&mut agent, "bg-1", None);
    let area = Rect::new(0, 0, 80, 30);
    let _ = draw_frame(agent, area);
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
fn app_with_running_subagent(child_session_id: &str) -> AppView {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = AppView::new(
        tx,
        crate::acp::model_state::ModelState::default(),
        Vec::new(),
        crate::render::draw::EscapeWriter::disconnected(),
    );
    let parent_id = AgentId(0);
    let child_id = AgentId(1);
    let mut parent = make_agent();
    parent.session.id = parent_id;
    parent.session.session_id = Some(agent_client_protocol::SessionId::new("parent"));
    parent.subagent_sessions.insert(
        child_session_id.to_owned(),
        super::test_fixtures::running_subagent_info(child_session_id),
    );
    app.agents.insert(parent_id, parent);
    app.active_view = ActiveView::Agent(parent_id);
    let mut child = super::test_fixtures::make_agent();
    child.session.session_id = Some(agent_client_protocol::SessionId::new(child_session_id));
    crate::app::session_views::test_support::link_child(
        &mut app.agents,
        parent_id,
        child_id,
        child,
        std::time::Instant::now(),
    );
    app
}
/// Dock subagent row: hovering reveals `[↗][stop]`; the painted `[stop]` must
/// kill the subagent and the painted `[↗]` must open it fullscreen.
#[test]
fn dock_subagent_icons_hover_and_click_where_painted() {
    crate::views::dock::set_enabled_for_test(true);
    let mut app = app_with_running_subagent("child-1");
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let area = Rect::new(0, 0, 80, 30);
    let buf = draw_frame(agent, area);
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
    let buf = draw_frame(agent, area);
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
    let _ = draw_frame(agent, area);
    let outcome = app.agents.get_mut(&AgentId(0)).unwrap().handle_mouse(&mouse(
        MouseEventKind::Down(MouseButton::Left),
        kill_x,
        row_y,
    ));
    assert!(
        matches!(outcome, InputOutcome::Action(Action::KillSubagent(ref id)) if id == "sa-child-1"),
        "clicking the painted [stop] must kill the subagent, got {outcome:?}"
    );
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, view_x, row_y));
    let _ = draw_frame(agent, area);
    let outcome = app.agents.get_mut(&AgentId(0)).unwrap().handle_mouse(&mouse(
        MouseEventKind::Down(MouseButton::Left),
        view_x,
        row_y,
    ));
    assert!(
        matches!(outcome, InputOutcome::Action(Action::OpenSession(ref sid)) if sid == "child-1"),
        "clicking the painted [↗] must request the child session, got {outcome:?}"
    );
}
/// Dock (remote `dock_enabled`): hovering a task row reveals `[↗][stop]`;
/// hovering and clicking the painted icons must act on that row.
#[test]
fn dock_icons_hover_and_click_where_painted() {
    crate::views::dock::set_enabled_for_test(true);
    let mut agent = make_agent();
    insert_running_task(&mut agent, "bg-1");
    let area = Rect::new(0, 0, 80, 30);
    let buf = draw_frame(agent, area);
    assert!(agent.dock_on, "dock must be on for this test");
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
    let _ = agent.handle_mouse(&mouse(MouseEventKind::Moved, area.x + 5, row_y));
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
    let _ = draw_frame(agent, area);
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
    let _ = draw_frame(agent, area);
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
fn ctrl_g_hides_and_shows_painted_dock() {
    crate::views::dock::set_enabled_for_test(true);
    let mut agent = make_agent();
    insert_running_task(&mut agent, "bg-1");
    let area = Rect::new(0, 0, 80, 30);
    let registry = ActionRegistry::defaults();
    let ctrl_g = Event::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('g'),
        KeyModifiers::CONTROL,
    ));
    let buf = draw_frame(&mut agent, area);
    assert!(agent.dock_shown);
    assert!(dock_row_has_tasks(&buf, agent.pane_areas.dock));
    assert!(matches!(
        agent.handle_input(&ctrl_g, &registry),
        InputOutcome::Changed
    ));
    let buf = draw_frame(&mut agent, area);
    assert!(agent.dock_on && agent.dock_hidden);
    assert!(!agent.dock_shown);
    assert_eq!(agent.pane_areas.dock.height, 0);
    assert!(!dock_row_has_tasks(&buf, agent.pane_areas.dock));
    assert!(matches!(
        agent.handle_input(&ctrl_g, &registry),
        InputOutcome::Changed
    ));
    let buf = draw_frame(&mut agent, area);
    assert!(agent.dock_shown && !agent.dock_hidden);
    assert!(dock_row_has_tasks(&buf, agent.pane_areas.dock));
}
