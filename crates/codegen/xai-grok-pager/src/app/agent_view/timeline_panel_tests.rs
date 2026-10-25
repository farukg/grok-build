//! Expanded timeline driven through the real input and draw entry points.
use crate::actions::ActionRegistry;
use crate::app::actions::Action;
use crate::app::agent_view::test_fixtures::{key, make_agent};
use crate::app::agent_view::{AgentView, AppRenderParams, BannerSlotParams};
use crate::app::app_view::InputOutcome;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::entry::{EntryId, ScrollbackEntry};
use crate::scrollback::render::ScratchBuffer;
use crate::scrollback::state::{TimelineRowKey, TimelineRowKind};
use crate::views::timeline_panel::gesture::BulkRequest;
use crate::views::timeline_panel::hint::FooterContext;
use crate::views::timeline_panel::{PanelFocus, TimelinePanelInput, timeline_panel_key};
use crate::views::tree_nav::{BulkAnchor, BulkScope};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 120,
    height: 30,
};

fn draw_frame(agent: &mut AgentView) {
    let _ = draw_buffer(agent);
}

fn draw_buffer(agent: &mut AgentView) -> Buffer {
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
    buf
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
    agent
        .scrollback
        .push_block(RenderBlock::thinking("pondering"));
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
        parent: Some(crate::app::agent::AgentId(0)),
        parent_session_id: "parent".into(),
        subagent_id: "child".into(),
        started_at: std::time::Instant::now(),
    });
    let area = AREA;
    assert!(matches!(
        press(&mut child, KeyCode::F(6)),
        InputOutcome::Changed
    ));
    draw_frame(&mut child);
    assert!(child.timeline_mode.panel().is_some());
    let mut buffer = Buffer::empty(area);
    let mut scratch = ScratchBuffer::new();
    child.draw(
        area,
        &mut buffer,
        &ActionRegistry::defaults(),
        &mut scratch,
        None,
        false,
        BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        AppRenderParams::default(),
    );
    let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        !text.trim().is_empty(),
        "timeline panel view is painted for child: {text}"
    );
}

#[test]
fn f6_toggles_timeline_panel() {
    let mut agent = conversation();
    let chat_width = agent.pane_areas.scrollback.width;
    press(&mut agent, KeyCode::F(6));
    draw_frame(&mut agent);
    let panel = agent
        .timeline_mode
        .panel()
        .expect("F6 expands the timeline");
    assert!(panel.area.width > 2, "panel is wider than the tick rail");

    press(&mut agent, KeyCode::F(6));
    draw_frame(&mut agent);
    assert!(
        agent.timeline_mode.panel().is_none(),
        "F6 folds back to the rail"
    );
    assert_eq!(agent.pane_areas.scrollback.width, chat_width);
}

#[test]
fn panel_arrows_visit_every_entry_including_tool_and_thinking() {
    let mut agent = conversation();
    open_panel_at_top(&mut agent);
    press(&mut agent, KeyCode::Char('*'));
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
    assert_eq!(
        agent.scrollback.selected(),
        Some(idx),
        "cursor selects its entry"
    );
    assert!(
        agent.scrollback.scroll_offset() > top,
        "chat scrolled to the cursor"
    );
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
    let first = agent
        .scrollback
        .index_of_id(first_id)
        .expect("first member");
    let members = agent
        .scrollback
        .span_at(first)
        .expect("folded")
        .range
        .clone();
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
    assert!(matches!(
        alt_click(&mut agent, answer),
        InputOutcome::Changed
    ));
    let id = agent
        .scrollback
        .iter_entries()
        .nth(answer)
        .map(|(id, _)| id);
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
    assert!(matches!(
        outcome,
        InputOutcome::Action(Action::CopyBlockContent)
    ));
    assert_eq!(agent.scrollback.selected(), Some(answer));
}

// Tree navigation, disclosure, gestures, colors, footer and toasts.
//
// `conversation()` outline (depth): 0 Turn Q1 (0), 1 thinking (1), 2 Group (1), 3-5 reads a/b/c (2),
// 6 answer (1), 7 Turn Q2 (0), 8 answer (1). The read run starts collapsed, so rows 3-5 are hidden.

fn press_with(agent: &mut AgentView, code: KeyCode, modifiers: KeyModifiers) -> InputOutcome {
    agent.handle_input(
        &Event::Key(KeyEvent::new(code, modifiers)),
        &ActionRegistry::defaults(),
    )
}

fn cursor_key(agent: &AgentView) -> TimelineRowKey {
    agent
        .timeline_mode
        .panel()
        .and_then(|panel| panel.cursor_row())
        .map(|row| row.key())
        .expect("cursor row")
}

fn go_to(agent: &mut AgentView, key: TimelineRowKey) {
    let panel = agent.timeline_mode.panel_mut().expect("panel open");
    assert!(
        panel.set_cursor_key(key) || panel.cursor_row().map(|row| row.key()) == Some(key),
        "row {key:?} is in the outline"
    );
}

fn id_at(ids: &[EntryId], index: usize) -> EntryId {
    ids.get(index).copied().expect("entry exists")
}

fn toast(agent: &AgentView) -> Option<&str> {
    agent.toast.as_ref().map(|(message, _)| message.as_str())
}

fn group_first(agent: &AgentView) -> EntryId {
    id_at(&all_ids(agent), 2)
}

fn group_expanded(agent: &AgentView) -> bool {
    let first = agent
        .scrollback
        .index_of_id(group_first(agent))
        .expect("group head");
    agent
        .scrollback
        .span_at(first)
        .expect("the read run folds")
        .expanded
}

fn turn_expanded(agent: &AgentView, prompt: EntryId) -> bool {
    agent
        .timeline_mode
        .panel()
        .expect("panel open")
        .turn_disclosure()
        .is_expanded(prompt)
}

fn panel_text(agent: &AgentView, buf: &Buffer) -> String {
    let area = agent.timeline_mode.panel().expect("panel open").area;
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol().to_string()))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Text of the left column outside the outline body (header and footer rows included).
fn column_text(agent: &AgentView, buf: &Buffer) -> String {
    let area = agent.timeline_mode.panel().expect("panel open").area;
    (0..buf.area.height)
        .map(|y| {
            (0..area.right())
                .filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol().to_string()))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn click(
    agent: &mut AgentView,
    visible_pos: u16,
    dx: u16,
    modifiers: KeyModifiers,
) -> InputOutcome {
    let area = agent.timeline_mode.panel().expect("panel open").area;
    agent.handle_input(
        &Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x + dx,
            row: area.y + visible_pos,
            modifiers,
        }),
        &ActionRegistry::defaults(),
    )
}

fn open_panel(agent: &mut AgentView) -> Vec<EntryId> {
    open_panel_at_top(agent);
    all_ids(agent)
}

#[test]
fn down_and_up_skip_rows_hidden_by_a_collapsed_group() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    let expected = [
        TimelineRowKey::Entry(id_at(&ids, 1)),
        TimelineRowKey::Group(id_at(&ids, 2)),
        TimelineRowKey::Entry(id_at(&ids, 5)),
        TimelineRowKey::Turn(id_at(&ids, 6)),
        TimelineRowKey::Entry(id_at(&ids, 7)),
    ];
    assert_eq!(cursor_key(&agent), TimelineRowKey::Turn(id_at(&ids, 0)));
    for key in expected {
        press(&mut agent, KeyCode::Down);
        assert_eq!(cursor_key(&agent), key);
    }
    press(&mut agent, KeyCode::Down);
    assert_eq!(
        cursor_key(&agent),
        expected.last().copied().expect("expected rows"),
        "↓ stops on the last row"
    );
    for key in expected.iter().rev().skip(1) {
        press(&mut agent, KeyCode::Up);
        assert_eq!(cursor_key(&agent), *key);
    }
    press(&mut agent, KeyCode::Up);
    press(&mut agent, KeyCode::Up);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Turn(id_at(&ids, 0)),
        "↑ stops on the first row"
    );
}

#[test]
fn vim_keys_move_like_the_arrows() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    press(&mut agent, KeyCode::Char('j'));
    press(&mut agent, KeyCode::Char('j'));
    assert_eq!(cursor_key(&agent), TimelineRowKey::Group(id_at(&ids, 2)));
    press(&mut agent, KeyCode::Char('l'));
    assert!(group_expanded(&agent), "l opens the group");
    press(&mut agent, KeyCode::Char('j'));
    assert_eq!(cursor_key(&agent), TimelineRowKey::Entry(id_at(&ids, 2)));
    press(&mut agent, KeyCode::Char('h'));
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Group(id_at(&ids, 2)),
        "h goes to the parent"
    );
    press(&mut agent, KeyCode::Char('k'));
    assert_eq!(cursor_key(&agent), TimelineRowKey::Entry(id_at(&ids, 1)));
}

#[test]
fn home_end_and_pages_move_over_visible_rows_only() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    press(&mut agent, KeyCode::End);
    assert_eq!(cursor_key(&agent), TimelineRowKey::Entry(id_at(&ids, 7)));
    press(&mut agent, KeyCode::Home);
    assert_eq!(cursor_key(&agent), TimelineRowKey::Turn(id_at(&ids, 0)));
    press(&mut agent, KeyCode::PageDown);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Entry(id_at(&ids, 7)),
        "a page that exceeds the 6 visible rows lands on the last one, not on hidden members"
    );
    press(&mut agent, KeyCode::PageUp);
    assert_eq!(cursor_key(&agent), TimelineRowKey::Turn(id_at(&ids, 0)));
}

#[test]
fn right_and_left_open_close_and_walk_the_tree() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);

    go_to(&mut agent, TimelineRowKey::Group(id_at(&ids, 2)));
    assert!(!group_expanded(&agent));
    press(&mut agent, KeyCode::Right);
    assert!(group_expanded(&agent), "→ on a collapsed group opens it");
    assert_eq!(cursor_key(&agent), TimelineRowKey::Group(id_at(&ids, 2)));
    press(&mut agent, KeyCode::Right);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Entry(id_at(&ids, 2)),
        "→ on an open group steps to its first member"
    );
    press(&mut agent, KeyCode::Right);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Entry(id_at(&ids, 2)),
        "→ on a leaf stays"
    );
    press(&mut agent, KeyCode::Left);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Group(id_at(&ids, 2)),
        "← on a leaf goes to the parent"
    );
    press(&mut agent, KeyCode::Left);
    assert!(!group_expanded(&agent), "← on an open group closes it");
    assert_eq!(cursor_key(&agent), TimelineRowKey::Group(id_at(&ids, 2)));
    press(&mut agent, KeyCode::Left);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Turn(id_at(&ids, 0)),
        "← on a closed group goes to its turn"
    );
    press(&mut agent, KeyCode::Left);
    assert!(
        !turn_expanded(&agent, id_at(&ids, 0)),
        "← on an open turn closes it"
    );
    assert_eq!(cursor_key(&agent), TimelineRowKey::Turn(id_at(&ids, 0)));
    press(&mut agent, KeyCode::Left);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Turn(id_at(&ids, 0)),
        "← on a closed root stays"
    );
    press(&mut agent, KeyCode::Right);
    assert!(
        turn_expanded(&agent, id_at(&ids, 0)),
        "→ on a closed turn opens it"
    );
    press(&mut agent, KeyCode::Right);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Entry(id_at(&ids, 1)),
        "→ on an open turn steps to its first child"
    );
}

#[test]
fn space_toggles_branches_and_explains_leaves() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Group(id_at(&ids, 2)));
    press(&mut agent, KeyCode::Char(' '));
    assert!(group_expanded(&agent));
    press(&mut agent, KeyCode::Char(' '));
    assert!(!group_expanded(&agent));

    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 6)));
    press(&mut agent, KeyCode::Char(' '));
    assert!(!turn_expanded(&agent, id_at(&ids, 6)));
    press(&mut agent, KeyCode::Char(' '));
    assert!(turn_expanded(&agent, id_at(&ids, 6)));

    go_to(&mut agent, TimelineRowKey::Entry(id_at(&ids, 1)));
    press(&mut agent, KeyCode::Char(' '));
    assert_eq!(
        toast(&agent),
        Some("An entry has no children to fold (e folds its content)")
    );
}

#[test]
fn collapsing_a_turn_hides_its_rows_and_keeps_the_cursor_visible() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    let before = draw_buffer(&mut agent);
    assert!(panel_text(&agent, &before).contains("Thinking pondering"));

    go_to(&mut agent, TimelineRowKey::Entry(id_at(&ids, 1)));
    agent
        .timeline_mode
        .panel_mut()
        .expect("panel")
        .set_turn_expanded(id_at(&ids, 0), false);
    let after = draw_buffer(&mut agent);
    let text = panel_text(&agent, &after);
    assert!(
        !text.contains("Thinking pondering"),
        "turn 1 children are gone: {text}"
    );
    assert!(text.contains("Turn 1") && text.contains("Turn 2"), "{text}");
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Turn(id_at(&ids, 0)),
        "the cursor moved to the folded turn"
    );
    press(&mut agent, KeyCode::Down);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Turn(id_at(&ids, 6)),
        "↓ skips the folded turn's rows"
    );
}

#[test]
fn chat_group_toggle_shows_in_the_panel() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    let hidden = draw_buffer(&mut agent);
    assert!(!panel_text(&agent, &hidden).contains("a.rs"));

    let first = agent
        .scrollback
        .index_of_id(id_at(&ids, 2))
        .expect("group head");
    let header = agent.scrollback.group_header_index(first).unwrap_or(first);
    agent.scrollback.set_selected(Some(header));
    assert!(agent.scrollback.toggle_group_expansion());
    let shown = draw_buffer(&mut agent);
    assert!(
        panel_text(&agent, &shown).contains("a.rs"),
        "members appear"
    );

    go_to(&mut agent, TimelineRowKey::Entry(id_at(&ids, 2)));
    press(&mut agent, KeyCode::Down);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Entry(id_at(&ids, 3)),
        "members are reachable once open"
    );
}

#[test]
fn shift_enter_toggles_all_siblings_and_flips_back() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 0)));
    press_with(&mut agent, KeyCode::Enter, KeyModifiers::SHIFT);
    assert!(
        turn_expanded(&agent, id_at(&ids, 0)) && !turn_expanded(&agent, id_at(&ids, 6)),
        "only the other turn closed"
    );
    assert_eq!(cursor_key(&agent), TimelineRowKey::Turn(id_at(&ids, 0)));
    press_with(&mut agent, KeyCode::Enter, KeyModifiers::SHIFT);
    assert!(
        turn_expanded(&agent, id_at(&ids, 0)) && turn_expanded(&agent, id_at(&ids, 6)),
        "both turns open again"
    );

    go_to(&mut agent, TimelineRowKey::Group(id_at(&ids, 2)));
    press_with(&mut agent, KeyCode::Enter, KeyModifiers::SHIFT);
    assert!(
        !group_expanded(&agent),
        "the anchor group has no branch sibling and stays closed"
    );
    press_with(&mut agent, KeyCode::Enter, KeyModifiers::SHIFT);
    assert!(!group_expanded(&agent));
}

#[test]
fn ctrl_enter_toggles_the_whole_subtree_and_flips_back() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 0)));
    press_with(&mut agent, KeyCode::Enter, KeyModifiers::CONTROL);
    assert!(
        group_expanded(&agent),
        "something below was closed, so the subtree opens"
    );
    assert!(turn_expanded(&agent, id_at(&ids, 0)));
    press_with(&mut agent, KeyCode::Enter, KeyModifiers::CONTROL);
    assert!(
        !group_expanded(&agent),
        "everything below was open, so the subtree closes"
    );
    assert!(
        turn_expanded(&agent, id_at(&ids, 0)),
        "the anchor turn stays open"
    );
    assert!(
        turn_expanded(&agent, id_at(&ids, 6)),
        "the other turn is outside the subtree"
    );
    assert_eq!(cursor_key(&agent), TimelineRowKey::Turn(id_at(&ids, 0)));
}

#[test]
fn star_toggles_every_branch_and_flips_back() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Entry(id_at(&ids, 7)));
    press(&mut agent, KeyCode::Char('*'));
    assert!(
        group_expanded(&agent),
        "a closed group exists, so * opens all"
    );
    assert!(turn_expanded(&agent, id_at(&ids, 0)) && turn_expanded(&agent, id_at(&ids, 6)));
    press(&mut agent, KeyCode::Char('*'));
    assert!(!group_expanded(&agent));
    assert!(
        !turn_expanded(&agent, id_at(&ids, 0)) && !turn_expanded(&agent, id_at(&ids, 6)),
        "all closed"
    );
    assert!(
        matches!(cursor_key(&agent), TimelineRowKey::Turn(_)),
        "the cursor moved off the hidden row onto its turn"
    );
    press(&mut agent, KeyCode::Char('*'));
    assert!(group_expanded(&agent));
    assert!(
        turn_expanded(&agent, id_at(&ids, 0)) && turn_expanded(&agent, id_at(&ids, 6)),
        "all open again"
    );
}

#[test]
fn plus_and_minus_force_the_siblings_open_or_closed() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 0)));
    press(&mut agent, KeyCode::Char('-'));
    assert!(!turn_expanded(&agent, id_at(&ids, 0)) && !turn_expanded(&agent, id_at(&ids, 6)));
    press(&mut agent, KeyCode::Char('-'));
    assert_eq!(
        toast(&agent),
        Some("Nothing to close here"),
        "a no-op says so"
    );
    press(&mut agent, KeyCode::Char('+'));
    assert!(turn_expanded(&agent, id_at(&ids, 0)) && turn_expanded(&agent, id_at(&ids, 6)));
}

#[test]
fn shift_arrows_open_and_close_the_subtree_below_the_cursor() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 0)));
    press_with(&mut agent, KeyCode::Right, KeyModifiers::SHIFT);
    assert!(group_expanded(&agent));
    press_with(&mut agent, KeyCode::Left, KeyModifiers::SHIFT);
    assert!(!group_expanded(&agent));
    assert!(
        turn_expanded(&agent, id_at(&ids, 0)),
        "the subtree gesture leaves the cursor row alone"
    );
}

#[test]
fn only_ctrl_enter_of_the_chords_reaches_the_panel() {
    let ctrl = |code| KeyEvent::new(code, KeyModifiers::CONTROL);
    for code in [
        KeyCode::Char('c'),
        KeyCode::Char('q'),
        KeyCode::Char('p'),
        KeyCode::Char('l'),
    ] {
        assert_eq!(
            timeline_panel_key(&ctrl(code)),
            TimelinePanelInput::Passthrough,
            "{code:?} stays global"
        );
    }
    assert_eq!(
        timeline_panel_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
        TimelinePanelInput::Passthrough
    );
    assert_eq!(
        timeline_panel_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )),
        TimelinePanelInput::Passthrough
    );
    assert_eq!(
        timeline_panel_key(&ctrl(KeyCode::Enter)),
        TimelinePanelInput::Bulk(BulkRequest::toggle(BulkScope::Subtree, BulkAnchor::Exclude))
    );
    assert_eq!(
        timeline_panel_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
        TimelinePanelInput::Bulk(BulkRequest::toggle(
            BulkScope::Siblings,
            BulkAnchor::Exclude
        ))
    );
    assert_eq!(
        timeline_panel_key(&KeyEvent::new(KeyCode::Char('*'), KeyModifiers::SHIFT)),
        TimelinePanelInput::Bulk(BulkRequest::toggle(BulkScope::All, BulkAnchor::Include)),
        "terminals report * with Shift"
    );
}

#[test]
fn click_on_the_glyph_toggles_and_elsewhere_only_selects() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    // visible rows: 0 Turn, 1 thinking, 2 Group (depth 1: glyph at column 2), 3 answer, 4 Turn, 5 answer
    click(&mut agent, 2, 8, KeyModifiers::NONE);
    assert_eq!(cursor_key(&agent), TimelineRowKey::Group(id_at(&ids, 2)));
    assert!(
        !group_expanded(&agent),
        "a click on the label only moves the cursor"
    );
    click(&mut agent, 2, 2, KeyModifiers::NONE);
    assert!(
        group_expanded(&agent),
        "a click on the glyph opens the group"
    );
    click(&mut agent, 2, 3, KeyModifiers::NONE);
    assert!(
        !group_expanded(&agent),
        "the gap after the glyph counts as the glyph"
    );

    click(&mut agent, 4, 0, KeyModifiers::NONE);
    assert!(
        !turn_expanded(&agent, id_at(&ids, 6)),
        "the turn's glyph closes it"
    );
    assert_eq!(cursor_key(&agent), TimelineRowKey::Turn(id_at(&ids, 6)));

    click(&mut agent, 1, 2, KeyModifiers::NONE);
    assert_eq!(cursor_key(&agent), TimelineRowKey::Entry(id_at(&ids, 1)));
    assert_eq!(toast(&agent), None, "a plain click on a leaf is silent");
}

#[test]
fn modifier_clicks_reach_the_rows_around_the_clicked_one() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    // Outline row 0 is Turn 0: every modifier gesture leaves it alone and reaches the rows around it.
    click(&mut agent, 0, 9, KeyModifiers::SHIFT);
    assert!(
        turn_expanded(&agent, id_at(&ids, 0)),
        "shift+click leaves the clicked row alone"
    );
    assert!(
        !turn_expanded(&agent, id_at(&ids, 6)),
        "shift+click toggles the siblings"
    );
    click(&mut agent, 0, 9, KeyModifiers::SHIFT);
    assert!(turn_expanded(&agent, id_at(&ids, 6)));
    click(&mut agent, 0, 9, KeyModifiers::CONTROL);
    assert!(
        group_expanded(&agent),
        "ctrl+click opens the rows below the clicked one"
    );
    assert!(turn_expanded(&agent, id_at(&ids, 0)));
    click(&mut agent, 0, 9, KeyModifiers::CONTROL);
    assert!(!group_expanded(&agent));
    assert!(
        turn_expanded(&agent, id_at(&ids, 0)),
        "the clicked turn keeps its state throughout"
    );
}

#[test]
fn hit_testing_follows_the_visible_rows() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    click(&mut agent, 3, 8, KeyModifiers::NONE);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Entry(id_at(&ids, 5)),
        "the row under the 4th line is the answer after the closed group, not a hidden read"
    );
    click(&mut agent, 2, 2, KeyModifiers::NONE);
    click(&mut agent, 3, 8, KeyModifiers::NONE);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Entry(id_at(&ids, 2)),
        "after opening, line 4 is the first read"
    );
}

#[test]
fn rows_carry_distinct_colors_per_class() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = conversation();
    open_panel(&mut agent);
    press(&mut agent, KeyCode::Char('*'));
    let buf = draw_buffer(&mut agent);
    let area = agent.timeline_mode.panel().expect("panel").area;
    // open outline: 0 Turn, 3 read (depth 2), 6 answer (depth 1); label starts after indent, glyph and a space
    let style_at = |pos: u16, depth: u16| {
        let cell = buf
            .cell((area.x + depth * 2 + 2, area.y + pos))
            .expect("label cell");
        (cell.fg, cell.modifier)
    };
    let turn = style_at(0, 0);
    let tool = style_at(3, 2);
    let answer = style_at(6, 1);
    assert_ne!(turn, tool);
    assert_ne!(turn, answer);
    assert_ne!(tool, answer);
}

#[test]
fn running_entries_are_marked() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = conversation();
    agent
        .scrollback
        .push(ScrollbackEntry::running(RenderBlock::agent_message(
            "working",
        )));
    draw_frame(&mut agent);
    open_panel(&mut agent);
    let buf = draw_buffer(&mut agent);
    let area = agent.timeline_mode.panel().expect("panel").area;
    // visible rows: 5 is the idle answer of turn 2, 6 the running one; both are depth 1
    let label = |pos: u16| {
        let cell = buf.cell((area.x + 4, area.y + pos)).expect("label cell");
        (cell.fg, cell.modifier)
    };
    assert_ne!(
        label(5),
        label(6),
        "a running agent message looks different from an idle one"
    );
}

#[test]
fn footer_lists_the_real_keys_and_gestures() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    let buf = draw_buffer(&mut agent);
    let text = column_text(&agent, &buf);
    for hint in [
        "Space fold",
        "Enter jump",
        "* fold all",
        "Shift+Enter toggle siblings",
        "Ctrl+Enter toggle subtree",
        "+/- open/close siblings",
        "R rewind",
    ] {
        assert!(text.contains(hint), "footer lacks {hint:?}:\n{text}");
    }
    assert!(
        !text.contains("d remove"),
        "an entry that cannot be removed does not advertise d"
    );

    go_to(&mut agent, TimelineRowKey::Group(id_at(&ids, 2)));
    let buf = draw_buffer(&mut agent);
    let on_group = column_text(&agent, &buf);
    assert!(
        !on_group.contains("y copy") && !on_group.contains("v viewer"),
        "{on_group}"
    );
    go_to(&mut agent, TimelineRowKey::Entry(id_at(&ids, 5)));
    let buf = draw_buffer(&mut agent);
    let on_entry = column_text(&agent, &buf);
    assert!(
        on_entry.contains("y copy") && on_entry.contains("v viewer"),
        "{on_entry}"
    );
}

#[test]
fn footer_advertises_remove_only_where_it_works() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    agent.session.session_id = Some("sess-1".to_string().into());
    if let Some(entry) = agent.scrollback.get_by_id_mut(id_at(&ids, 0))
        && let RenderBlock::UserPrompt(prompt) = &mut entry.block
    {
        prompt.prompt_index = Some(0);
    }
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 0)));
    let buf = draw_buffer(&mut agent);
    let numbered = column_text(&agent, &buf);
    assert!(numbered.contains("d remove"), "{numbered}");
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 6)));
    let buf = draw_buffer(&mut agent);
    let unnumbered = column_text(&agent, &buf);
    assert!(!unnumbered.contains("d remove"), "{unnumbered}");
}

#[test]
fn a_group_row_offers_remove_while_one_member_can_go() {
    use crate::app::agent_view::removal_test_support::{
        push_numbered_prompt, push_registered_read,
    };
    let _theme = crate::theme::cache::pin_theme();
    crate::appearance::cache::set_group_tool_verbs(true);
    let mut agent = make_agent();
    agent.session.session_id = Some("sess-1".to_string().into());
    push_numbered_prompt(&mut agent, "Q1", 0);
    let first = push_registered_read(&mut agent, "call-a", "a.rs");
    push_registered_read(&mut agent, "call-b", "b.rs");
    push_registered_read(&mut agent, "call-c", "c.rs");
    agent.scrollback.push_block(tall("first"));
    draw_frame(&mut agent);
    agent.scrollback.goto_top();
    draw_frame(&mut agent);
    open_panel(&mut agent);

    go_to(&mut agent, TimelineRowKey::Group(first));
    let buf = draw_buffer(&mut agent);
    assert!(
        column_text(&agent, &buf).contains("d remove"),
        "a group of removable tool calls offers d"
    );

    agent.session.session_id = None;
    let buf = draw_buffer(&mut agent);
    assert!(
        !column_text(&agent, &buf).contains("d remove"),
        "losing the session withdraws the hint although the group did not change"
    );
}

#[test]
fn footer_fits_a_narrow_column() {
    let _theme = crate::theme::cache::pin_theme();
    let theme = crate::theme::Theme::current();
    for width in [14u16, 22, 30, 42] {
        let lines = crate::views::timeline_panel::hint::footer_lines(
            width,
            &theme,
            &FooterContext::widest(),
        );
        assert!(!lines.is_empty(), "width {width} shows some hints");
        for line in &lines {
            assert!(
                line.width() <= usize::from(width.saturating_sub(2)),
                "width {width}: {line:?}"
            );
        }
    }
}

#[test]
fn remove_keeps_the_focus_in_the_panel() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    press(&mut agent, KeyCode::Char('d'));
    assert_eq!(
        agent.timeline_mode.panel().expect("panel").focus,
        PanelFocus::Panel
    );
    press(&mut agent, KeyCode::Down);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Entry(id_at(&ids, 1)),
        "the panel still drives the cursor after d"
    );
}

#[test]
fn actions_without_effect_say_why() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Group(id_at(&ids, 2)));
    for key in ['y', 'Y', 'v'] {
        agent.toast = None;
        let outcome = press(&mut agent, KeyCode::Char(key));
        assert!(matches!(outcome, InputOutcome::Changed), "{key}");
        assert_eq!(toast(&agent), Some("Select a single entry first"), "{key}");
    }
    agent.toast = None;
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 0)));
    press(&mut agent, KeyCode::Char('e'));
    assert_eq!(toast(&agent), Some("This entry has nothing to fold"));
}

#[test]
fn chat_following_a_hidden_row_parks_on_its_folded_ancestor() {
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    press(&mut agent, KeyCode::Tab);
    agent
        .timeline_mode
        .panel_mut()
        .expect("panel")
        .set_turn_expanded(id_at(&ids, 0), false);
    agent.scrollback.set_selected(Some(1));
    draw_frame(&mut agent);
    assert_eq!(cursor_key(&agent), TimelineRowKey::Turn(id_at(&ids, 0)));
    let builds = agent.timeline_mode.panel().expect("panel").view_builds();
    for _ in 0..5 {
        draw_frame(&mut agent);
    }
    assert_eq!(
        agent.timeline_mode.panel().expect("panel").view_builds(),
        builds
    );
}

fn number_prompts(agent: &mut AgentView) {
    let ids = all_ids(agent);
    for (id, ordinal) in [(id_at(&ids, 0), 3), (id_at(&ids, 6), 4)] {
        if let Some(entry) = agent.scrollback.get_by_id_mut(id)
            && let RenderBlock::UserPrompt(prompt) = &mut entry.block
        {
            prompt.prompt_index = Some(ordinal);
        }
    }
}

fn large_agent() -> AgentView {
    let mut agent = make_agent();
    agent.scrollback.begin_batch();
    for turn in 0..500 {
        agent
            .scrollback
            .push_block(RenderBlock::user_prompt(format!("Q{turn}")));
        for _ in 0..39 {
            agent.scrollback.push_block(RenderBlock::agent_message("x"));
        }
    }
    agent.scrollback.end_batch();
    draw_frame(&mut agent);
    agent.scrollback.goto_top();
    draw_frame(&mut agent);
    agent
}

#[test]
fn a_large_outline_moves_without_rebuilding_its_view() {
    let mut agent = large_agent();
    let ids = all_ids(&agent);
    assert_eq!(ids.len(), 20_000);
    press(&mut agent, KeyCode::F(6));
    draw_frame(&mut agent);
    press(&mut agent, KeyCode::Home);
    draw_frame(&mut agent);
    assert_eq!(cursor_key(&agent), TimelineRowKey::Turn(id_at(&ids, 0)));

    let builds = agent.timeline_mode.panel().expect("panel").view_builds();
    for step in 1..=10 {
        press(&mut agent, KeyCode::Down);
        draw_frame(&mut agent);
        assert_eq!(cursor_key(&agent), TimelineRowKey::Entry(id_at(&ids, step)));
    }
    assert_eq!(
        agent.timeline_mode.panel().expect("panel").view_builds(),
        builds,
        "moving the cursor reuses the cached view"
    );

    press(&mut agent, KeyCode::End);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Entry(id_at(&ids, 19_999))
    );
    press(&mut agent, KeyCode::Home);
    press(&mut agent, KeyCode::Left);
    draw_frame(&mut agent);
    assert_eq!(
        agent.timeline_mode.panel().expect("panel").view_builds(),
        builds + 1,
        "closing a turn rebuilds the view exactly once"
    );
    press(&mut agent, KeyCode::Down);
    assert_eq!(
        cursor_key(&agent),
        TimelineRowKey::Turn(id_at(&ids, 40)),
        "↓ skips the 39 hidden entries"
    );
}
#[path = "timeline_panel_persistence_tests.rs"]
mod persistence;
