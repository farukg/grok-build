//! Expanded timeline: the rail unfolded into a navigable outline of every scrollback entry (F6 / palette "Timeline").
//!
//! Rows come from [`ScrollbackState::timeline_outline`] and are rebuilt only when its generation moves, so streaming and scrolling cost nothing here.
//! The tree view over them (which rows are visible) is rebuilt only when the generation or the turn disclosure moves.
//! Rendering labels only the visible rows inside the panel window.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::tool::ToolCallBlock;
use crate::scrollback::entry::ScrollbackEntry;
use crate::scrollback::state::groups::GroupKind;
use crate::scrollback::state::verb_group::{truncation_header_label, verb_group_header_label};
use crate::scrollback::state::{
    ScrollbackState, TimelineRow, TimelineRowKey, TimelineRowKind, prompt_preview,
};
use crate::theme::Theme;
use crate::views::row_style::{OutlineClass, RowState};
use disclosure::TurnDisclosure;
use footer_memo::FooterMemo;
use gesture::{BulkRequest, PanelNav, bulk_request_for};
use outline_nav::ParkedFollow;
use outline_view::OutlineView;
use row_line::{RowLabel, outline_line};

pub mod disclosure;
pub mod footer_memo;
pub mod gesture;
pub mod hint;
mod outline_nav;
mod outline_view;
mod row_line;

/// How the timeline column presents itself.
#[derive(Debug, Default)]
pub enum TimelineMode {
    /// The per-turn tick rail (subject to the `show_timeline` setting).
    #[default]
    Rail,
    Expanded(TimelinePanelState),
}

impl TimelineMode {
    pub fn panel(&self) -> Option<&TimelinePanelState> {
        match self {
            TimelineMode::Rail => None,
            TimelineMode::Expanded(panel) => Some(panel),
        }
    }

    pub fn panel_mut(&mut self) -> Option<&mut TimelinePanelState> {
        match self {
            TimelineMode::Rail => None,
            TimelineMode::Expanded(panel) => Some(panel),
        }
    }
}

/// Which side drives the shared selection; sync runs one way only, so the two never feed back into each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    /// Keys move the panel cursor; the chat scrolls to it.
    Panel,
    /// The chat is navigated; the cursor follows the chat selection or viewport top.
    Chat,
}

/// An action on the cursor row, dispatched through the existing scrollback action for the row's entry.
/// On a group row it applies to the group as a whole (fold, rewind to its first member).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineItemAction {
    Jump,
    ToggleFold,
    CopyContent,
    CopyMeta,
    OpenViewer,
    Rewind,
    Remove,
}

#[derive(Debug)]
pub struct TimelinePanelState {
    rows: Vec<TimelineRow>,
    /// `outline_generation` the rows were built at.
    generation: u64,
    cursor: usize,
    scroll: usize,
    pub focus: PanelFocus,
    /// Rendered panel rect; empty while the layout has no room for the panel.
    pub area: Rect,
    /// Folded turns; group rows read their state from the scrollback instead.
    turns: TurnDisclosure,
    /// Moves with every change of `turns`; with `generation` it keys `view`.
    turns_version: u64,
    view: OutlineView,
    parked: Option<ParkedFollow>,
    footer: FooterMemo,
    #[cfg(test)]
    view_builds: usize,
}

impl TimelinePanelState {
    /// Build the outline with the cursor on the entry at `at` (or the first row).
    pub fn open(scrollback: &ScrollbackState, at: Option<usize>) -> Self {
        let mut panel = Self {
            rows: scrollback.timeline_outline(),
            generation: scrollback.outline_generation(),
            cursor: 0,
            scroll: 0,
            focus: PanelFocus::Panel,
            area: Rect::default(),
            turns: TurnDisclosure::default(),
            turns_version: 0,
            view: OutlineView::default(),
            parked: None,
            footer: FooterMemo::default(),
            #[cfg(test)]
            view_builds: 0,
        };
        if let Some(key) = at.and_then(|idx| scrollback.timeline_row_key(idx)) {
            panel.set_cursor_key(key);
        }
        panel.refresh_view(scrollback);
        panel
    }

    /// Rebuild the rows when the outline changed shape, keeping the cursor on the same row.
    pub fn refresh(&mut self, scrollback: &ScrollbackState) {
        let generation = scrollback.outline_generation();
        if generation == self.generation {
            self.refresh_view(scrollback);
            return;
        }
        let key = self.cursor_row().map(TimelineRow::key);
        self.rows = scrollback.timeline_outline();
        self.generation = generation;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        if let Some(key) = key {
            self.set_cursor_key(key);
        }
        self.refresh_view(scrollback);
    }

    pub fn cursor_row(&self) -> Option<&TimelineRow> {
        self.rows.get(self.cursor)
    }

    pub fn set_cursor_key(&mut self, key: TimelineRowKey) -> bool {
        match self.rows.iter().position(|row| row.key() == key) {
            Some(pos) => self.set_cursor(pos),
            None => false,
        }
    }

    /// Point the cursor at `key` unless it is already there (the common per-frame case costs no scan).
    /// A target hidden inside a folded branch parks the cursor on the branch; that costs no new scan until something changes.
    pub fn follow(&mut self, key: TimelineRowKey) {
        if self.cursor_row().map(TimelineRow::key) != Some(key) && !self.is_parked_on(key) {
            self.set_cursor_key(key);
            self.park_if_hidden(key);
        }
    }

    pub fn set_cursor(&mut self, pos: usize) -> bool {
        let pos = pos.min(self.rows.len().saturating_sub(1));
        let changed = pos != self.cursor;
        self.cursor = pos;
        changed
    }

    /// Move the cursor by `delta` visible rows (folded-away rows are skipped).
    pub fn move_cursor(&mut self, delta: isize) -> bool {
        let Some(position) = self.view.position_of(self.cursor) else {
            return false;
        };
        let target = position.saturating_add_signed(delta);
        match self
            .view
            .row_at(target.min(self.view.visible().len().saturating_sub(1)))
        {
            Some(row) => self.set_cursor(row),
            None => false,
        }
    }

    /// Keep the cursor inside the rendered window; the scroll offset counts visible rows.
    pub fn scroll_into_view(&mut self) {
        let height = self.area.height as usize;
        if height == 0 {
            return;
        }
        if let Some(position) = self.view.position_of(self.cursor) {
            if position < self.scroll {
                self.scroll = position;
            } else if position >= self.scroll + height {
                self.scroll = position + 1 - height;
            }
        }
        self.scroll = self
            .scroll
            .min(self.view.visible().len().saturating_sub(height));
    }

    /// Outline row under a screen position.
    pub fn row_at(&self, col: u16, row: u16) -> Option<usize> {
        if !self.area.contains((col, row).into()) {
            return None;
        }
        self.view.row_at(self.scroll + (row - self.area.y) as usize)
    }
}

/// What a key does while the panel owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelinePanelInput {
    /// Tree navigation over the visible rows, including Left/Right opening and closing.
    Nav(PanelNav),
    /// Space: open or close the branch under the cursor.
    Toggle,
    Bulk(BulkRequest),
    Act(TimelineItemAction),
    /// `g`: flip where panel settings are saved, global or this session (shared with F7).
    ToggleSaveTarget,
    /// `t`: new turns start open or folded.
    ToggleTurnDefault,
    FocusChat,
    Close,
    /// An unbound plain key, swallowed so it cannot reach the panes behind the panel.
    Consumed,
    /// A chord (Ctrl/Alt/Super): falls through to the normal routing (palette, cancel, quit).
    Passthrough,
}

pub fn timeline_panel_key(key: &KeyEvent) -> TimelinePanelInput {
    if key.kind == KeyEventKind::Release {
        return TimelinePanelInput::Consumed;
    }
    // Ctrl+Enter is the one chord the panel owns; every other Ctrl/Alt/Super chord stays global
    if let Some(request) = bulk_request_for(key) {
        return TimelinePanelInput::Bulk(request);
    }
    if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
        return TimelinePanelInput::Passthrough;
    }
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => TimelinePanelInput::Nav(PanelNav::Down),
        KeyCode::Up | KeyCode::Char('k') => TimelinePanelInput::Nav(PanelNav::Up),
        KeyCode::PageDown => TimelinePanelInput::Nav(PanelNav::PageDown),
        KeyCode::PageUp => TimelinePanelInput::Nav(PanelNav::PageUp),
        KeyCode::Home => TimelinePanelInput::Nav(PanelNav::Home),
        KeyCode::End => TimelinePanelInput::Nav(PanelNav::End),
        KeyCode::Left | KeyCode::Char('h') => TimelinePanelInput::Nav(PanelNav::Left),
        KeyCode::Right | KeyCode::Char('l') => TimelinePanelInput::Nav(PanelNav::Right),
        KeyCode::Char(' ') => TimelinePanelInput::Toggle,
        KeyCode::Enter | KeyCode::Char('f') => TimelinePanelInput::Act(TimelineItemAction::Jump),
        KeyCode::Char('e') => TimelinePanelInput::Act(TimelineItemAction::ToggleFold),
        KeyCode::Char('y') => TimelinePanelInput::Act(TimelineItemAction::CopyContent),
        KeyCode::Char('Y') => TimelinePanelInput::Act(TimelineItemAction::CopyMeta),
        KeyCode::Char('v') => TimelinePanelInput::Act(TimelineItemAction::OpenViewer),
        KeyCode::Char('R') => TimelinePanelInput::Act(TimelineItemAction::Rewind),
        KeyCode::Char('d') | KeyCode::Delete => TimelinePanelInput::Act(TimelineItemAction::Remove),
        KeyCode::Char('g') => TimelinePanelInput::ToggleSaveTarget,
        KeyCode::Char('t') => TimelinePanelInput::ToggleTurnDefault,
        KeyCode::Tab => TimelinePanelInput::FocusChat,
        KeyCode::Esc | KeyCode::F(6) => TimelinePanelInput::Close,
        KeyCode::F(_) => TimelinePanelInput::Passthrough,
        _ => TimelinePanelInput::Consumed,
    }
}

/// Paint the panel: one row per visible outline row in the window.
pub fn render_panel(
    buf: &mut Buffer,
    panel: &TimelinePanelState,
    scrollback: &ScrollbackState,
    theme: &Theme,
) {
    let area = panel.area;
    if area.width < 3 || area.height == 0 {
        return;
    }
    let bg = theme.bg_base;
    ratatui::widgets::Widget::render(ratatui::widgets::Clear, area, buf);
    buf.set_style(area, Style::default().bg(bg));
    let text_x = area.x;
    let text_w = area.width;
    let cursor_style = match panel.focus {
        PanelFocus::Panel => theme.selection_overlay(),
        PanelFocus::Chat => theme.hover_overlay(),
    };
    let window = panel
        .view
        .visible()
        .iter()
        .skip(panel.scroll)
        .take(area.height as usize);
    for (offset, &index) in window.enumerate() {
        let (Some(row), Some(tree)) = (panel.rows.get(index), panel.view.tree().get(index)) else {
            continue;
        };
        let y = area.y + offset as u16;
        let label = row_label(row, scrollback, theme);
        let line = outline_line(row.depth, tree.branch, label, theme, text_w as usize);
        buf.set_line(text_x, y, &line, text_w);
        if index == panel.cursor {
            buf.set_style(Rect::new(text_x, y, text_w, 1), cursor_style);
        }
    }
}

fn row_label(row: &TimelineRow, scrollback: &ScrollbackState, theme: &Theme) -> RowLabel {
    match row.kind {
        TimelineRowKind::Turn {
            turn_idx,
            prompt_id,
        } => {
            let preview = scrollback
                .get_by_id(prompt_id)
                .map(entry_label)
                .unwrap_or_default();
            RowLabel::plain(
                OutlineClass::Turn,
                format!("Turn {} {preview}", turn_idx + 1),
            )
        }
        TimelineRowKind::Group { first_id } => {
            let text = scrollback
                .index_of_id(first_id)
                .and_then(|idx| group_label(scrollback, idx, theme))
                .unwrap_or_default();
            RowLabel::plain(OutlineClass::Group, text)
        }
        TimelineRowKind::Entry { id } => match scrollback.get_by_id(id) {
            Some(entry) => RowLabel {
                class: OutlineClass::of_block(&entry.block),
                state: RowState {
                    running: entry.is_running,
                    ..RowState::default()
                },
                text: entry_label(entry),
            },
            None => RowLabel::plain(OutlineClass::SystemNote, String::new()),
        },
    }
}

/// Header label of the fold span starting at `first_idx`, worded like the chat's own header row.
fn group_label(scrollback: &ScrollbackState, first_idx: usize, theme: &Theme) -> Option<String> {
    let span = scrollback.span_at(first_idx)?;
    let members = scrollback.entries_in_range(span.range.clone());
    let show_thinking = crate::appearance::cache::load_show_thinking_blocks();
    Some(match span.kind {
        GroupKind::VerbRun { .. } => {
            verb_group_header_label(&members, 0, members.len(), show_thinking, theme).text
        }
        GroupKind::Truncation { participants, .. } => {
            truncation_header_label(&members, 0..members.len(), None, show_thinking, theme)
                .map_or_else(|| format!("{participants} entries"), |label| label.text)
        }
    })
}

/// One-line label for an entry, from stored source fields only (no layout, no rendering).
fn entry_label(entry: &ScrollbackEntry) -> String {
    match &entry.block {
        RenderBlock::Stub(b) => prompt_preview(&b.text),
        RenderBlock::UserPrompt(b) => prompt_preview(&b.text),
        RenderBlock::AgentMessage(b) => b.content().preview(),
        RenderBlock::Thinking(b) => format!("Thinking {}", b.content().preview()),
        RenderBlock::ToolCall(tool) => {
            let verb = tool
                .label_kind()
                .map_or("Tool", |kind| kind.verb(entry.is_running));
            format!("{verb} {}", prompt_preview(tool_subject(tool)))
        }
        RenderBlock::System(b) => prompt_preview(&b.text),
        RenderBlock::SessionEvent(b) => prompt_preview(&b.event.message()),
        RenderBlock::BgTask(b) => prompt_preview(b.description.as_deref().unwrap_or(&b.command)),
        RenderBlock::Subagent(b) => format!("Subagent {}", prompt_preview(&b.description)),
        RenderBlock::Workflow(b) => format!("Workflow {}", prompt_preview(&b.name)),
        RenderBlock::Btw(b) => format!("btw {}", prompt_preview(&b.question)),
        RenderBlock::ContextInfo(b) => format!("Context {}", b.model),
        RenderBlock::MemoryCapture(_) => "Memory capture".to_string(),
        RenderBlock::Injected(b) => format!("{} {}", b.label, prompt_preview(&b.text)),
    }
}

fn tool_subject(tool: &ToolCallBlock) -> &str {
    match tool {
        ToolCallBlock::Execute(b) => &b.command,
        ToolCallBlock::Read(b) => &b.path,
        ToolCallBlock::Edit(b) => &b.path,
        ToolCallBlock::ListDir(b) => &b.path,
        ToolCallBlock::Search(b) => &b.pattern,
        ToolCallBlock::WebFetch(b) => &b.url,
        ToolCallBlock::WebSearch(b) => &b.query,
        ToolCallBlock::IntegrationSearch(b) => &b.query,
        ToolCallBlock::UseTool(b) => &b.tool_name,
        ToolCallBlock::MemorySearch(b) => &b.query,
        ToolCallBlock::SentMessage(b) => b.input.as_ref().map_or("", |input| input.text.as_str()),
        ToolCallBlock::Skill(b) | ToolCallBlock::Other(b) => &b.name,
    }
}
