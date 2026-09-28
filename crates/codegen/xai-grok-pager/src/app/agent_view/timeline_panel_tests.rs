//! Expanded timeline driven through the real input and draw entry points.
use crate::actions::ActionRegistry;
use crate::app::actions::Action;
use crate::app::agent_view::test_fixtures::{key, make_agent};
use crate::app::agent_view::{AgentView, AppRenderParams, BannerSlotParams};
use crate::app::app_view::InputOutcome;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::entry::EntryId;
use crate::scrollback::render::ScratchBuffer;
use crate::scrollback::state::TimelineRowKind;
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 120,
    height: 30,
};

fn draw_frame(agent: &mut AgentView) {
    let mut buf = Buffer::empty(AREA);
    let mut scratch = ScratchBuffer::new();
    let _ = agent.draw(
        AREA,
        &mut buf,
        &ActionRegistry::defaults(),
        &mut scratch,
        None,
        false,
        BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        AppRenderParams::default(),
    );
}

fn press(agent: &mut AgentView, code: KeyCode) -> InputOutcome {
    agent.handle_input(&key(code), &ActionRegistry::defaults())
}

fn tall(label: &str) -> RenderBlock {
    RenderBlock::agent_message(
        (0..12)
            .map(|i| format!("{label} paragraph {i}"))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

/// Two turns: prompt, thinking, a foldable run of reads, and tall answers so the chat scrolls.
fn conversation() -> AgentView {
    crate::appearance::cache::set_group_tool_verbs(true);
    crate::appearance::cache::set_show_thinking_blocks(true);
    let mut agent = make_agent();
    agent.scrollback.push_block(RenderBlock::user_prompt("Q1"));
    agent.scrollback.push_block(RenderBlock::thinking("pondering"));
    for path in ["a.rs", "b.rs", "c.rs"] {
        agent.scrollback.push_block(RenderBlock::read(path, None));
    }
    agent.scrollback.push_block(tall("first"));
    agent.scrollback.push_block(RenderBlock::user_prompt("Q2"));
    agent.scrollback.push_block(tall("second"));
    draw_frame(&mut agent);
    agent.scrollback.goto_top();
    draw_frame(&mut agent);
    agent
}

fn all_ids(agent: &AgentView) -> Vec<EntryId> {
    agent.scrollback.iter_entries().map(|(id, _)| id).collect()
}

fn cursor_kind(agent: &AgentView) -> Option<TimelineRowKind> {
    agent
        .timeline_mode
        .panel()
        .and_then(|panel| panel.cursor_row())
        .map(|row| row.kind)
}

fn cursor_entry(agent: &AgentView) -> Option<EntryId> {
    agent
        .timeline_mode
        .panel()
        .and_then(|panel| panel.cursor_row())
        .map(|row| row.entry_id())
}

fn open_panel_at_top(agent: &mut AgentView) {
    assert!(matches!(press(agent, KeyCode::F(6)), InputOutcome::Changed));
    draw_frame(agent);
    for _ in 0..4 {
        press(agent, KeyCode::PageUp);
    }
}

fn alt_click(agent: &mut AgentView, idx: usize) -> InputOutcome {
    let (rect, _, _) = agent
        .scrollback
        .entry_screen_area(idx, agent.pane_areas.scrollback)
        .expect("entry on screen");
    agent.handle_input(
        &Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 1,
            row: rect.y,
            modifiers: KeyModifiers::ALT,
        }),
        &ActionRegistry::defaults(),
    )
}

#[test]
fn f6_opens_timeline_in_child() {
    let mut child = conversation();
    child.role = crate::app::agent_view::AgentRole::Child(crate::app::agent_view::ChildLink {
        parent: crate::app::agent::AgentId(0),
        parent_session_id: "parent".into(),
        subagent_id: "child".into(),
        started_at: std::time::Instant::now(),
    });
    let area = AREA;
    assert!(matches!(press(&mut child, KeyCode::F(6)), InputOutcome::Changed));
    draw_frame(&mut child);
    assert!(child.timeline_mode.panel().is_some());
    let mut buffer = Buffer::empty(area);
    let mut scratch = ScratchBuffer::new();
    child.draw(area, &mut buffer, &ActionRegistry::defaults(), &mut scratch, None, false, BannerSlotParams::none(), false, &mut Vec::new(), AppRenderParams::default());
    let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(!text.trim().is_empty(), "timeline panel view is painted for child: {text}");
}

#[test]
#[test]
fn f6_toggles_timeline_panel() {
    let mut agent = conversation();
    let chat_width = agent.pane_areas.scrollback.width;
    press(&mut agent, KeyCode::F(6));
    draw_frame(&mut agent);
    let panel = agent.timeline_mode.panel().expect("F6 expands the timeline");
    assert!(panel.area.width > 2, "panel is wider than the tick rail");

    press(&mut agent, KeyCode::F(6));
    draw_frame(&mut agent);
    assert!(agent.timeline_mode.panel().is_none(), "F6 folds back to the rail");
    assert_eq!(agent.pane_areas.scrollback.width, chat_width);
}

#[test]
fn panel_arrows_visit_every_entry_including_tool_and_thinking() {
    let mut agent = conversation();
    open_panel_at_top(&mut agent);
    let mut visited = vec![cursor_entry(&agent).expect("cursor")];
    for _ in 0..all_ids(&agent).len() * 2 {
        press(&mut agent, KeyCode::Down);
        visited.extend(cursor_entry(&agent));
    }
    for id in all_ids(&agent) {
        assert!(visited.contains(&id), "{id:?} never reached by ↓");
    }
}

#[test]
fn panel_cursor_down_scrolls_scrollback() {
    let mut agent = conversation();
    open_panel_at_top(&mut agent);
    let top = agent.scrollback.scroll_offset();
    let last = *all_ids(&agent).last().expect("entries");
    for _ in 0..all_ids(&agent).len() * 2 {
        if cursor_entry(&agent) == Some(last) {
            break;
        }
        press(&mut agent, KeyCode::Down);
    }
    assert_eq!(cursor_entry(&agent), Some(last), "↓ reaches the last entry");
    let idx = agent.scrollback.index_of_id(last).expect("last entry");
    assert_eq!(agent.scrollback.selected(), Some(idx), "cursor selects its entry");
    assert!(agent.scrollback.scroll_offset() > top, "chat scrolled to the cursor");
}

#[test]
fn scrollback_scroll_moves_unfocused_panel_cursor() {
    let mut agent = conversation();
    open_panel_at_top(&mut agent);
    press(&mut agent, KeyCode::Tab);
    draw_frame(&mut agent);
    let before = cursor_entry(&agent);
    let chat = agent.pane_areas.scrollback;
    for _ in 0..10 {
        agent.handle_scroll(3, chat.x + 1, chat.y + 1);
    }
    draw_frame(&mut agent);
    let after = cursor_entry(&agent).expect("cursor");
    assert_ne!(Some(after), before, "wheel in the chat moved the cursor");
    let idx = agent.scrollback.index_of_id(after).expect("entry");
    assert!(
        agent.scrollback.entry_screen_area(idx, chat).is_some(),
        "the cursor tracks an entry that is on screen"
    );
}

#[test]
fn group_row_expand_toggles_scrollback_group() {
    let mut agent = conversation();
    open_panel_at_top(&mut agent);
    for _ in 0..all_ids(&agent).len() {
        if matches!(cursor_kind(&agent), Some(TimelineRowKind::Group { .. })) {
            break;
        }
        press(&mut agent, KeyCode::Down);
    }
    let Some(TimelineRowKind::Group { first_id }) = cursor_kind(&agent) else {
        panic!("the read run is listed as a group row");
    };
    let first = agent.scrollback.index_of_id(first_id).expect("first member");
    let members = agent.scrollback.span_at(first).expect("folded").range.clone();
    assert!(members.len() >= 3);

    press(&mut agent, KeyCode::Right);
    draw_frame(&mut agent);
    assert!(agent.scrollback.span_at(first).is_some_and(|s| s.expanded));
    for idx in members.clone() {
        assert!(
            agent.scrollback.get_cached_entry_height(idx).unwrap_or(0) > 0,
            "member {idx} shown in the chat"
        );
    }
    assert!(
        matches!(cursor_kind(&agent), Some(TimelineRowKind::Group { .. })),
        "cursor stays on the group row"
    );

    press(&mut agent, KeyCode::Left);
    draw_frame(&mut agent);
    assert!(agent.scrollback.span_at(first).is_some_and(|s| !s.expanded));
}

#[test]
fn alt_click_opens_panel_on_clicked_entry() {
    let mut agent = conversation();
    let answer = 5;
    assert!(matches!(alt_click(&mut agent, answer), InputOutcome::Changed));
    let id = agent.scrollback.iter_entries().nth(answer).map(|(id, _)| id);
    assert_eq!(cursor_entry(&agent), id);
}

#[test]
fn removing_unidentified_entry_is_disabled_without_scrollback_mutation() {
    let mut agent = conversation();
    open_panel_at_top(&mut agent);
    let before = all_ids(&agent);
    let outcome = press(&mut agent, KeyCode::Char('d'));
    assert!(matches!(outcome, InputOutcome::Changed));
    assert_eq!(all_ids(&agent), before);
    assert!(agent.pending_effects.is_empty());
}

#[test]
fn panel_copy_dispatches_copy_for_cursor_entry() {
    let mut agent = conversation();
    let answer = 5;
    alt_click(&mut agent, answer);
    let outcome = press(&mut agent, KeyCode::Char('y'));
    assert!(matches!(outcome, InputOutcome::Action(Action::CopyBlockContent)));
    assert_eq!(agent.scrollback.selected(), Some(answer));
}
