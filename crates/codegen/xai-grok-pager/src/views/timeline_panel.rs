//! Expanded timeline: the rail unfolded into a navigable outline of every scrollback entry (F6 / palette "Timeline").
//!
//! Rows come from [`ScrollbackState::timeline_outline`] and are rebuilt only when its generation moves, so streaming and scrolling cost nothing here.
//! Rendering labels only the rows inside the panel window.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::render::line_utils::truncate_str;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::tool::ToolCallBlock;
use crate::scrollback::entry::ScrollbackEntry;
use crate::scrollback::state::groups::GroupKind;
use crate::scrollback::state::verb_group::{truncation_header_label, verb_group_header_label};
use crate::scrollback::state::{
    ScrollbackState, TimelineRow, TimelineRowKey, TimelineRowKind, prompt_preview,
};
use crate::theme::Theme;

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
        };
        if let Some(key) = at.and_then(|idx| scrollback.timeline_row_key(idx)) {
            panel.set_cursor_key(key);
        }
        panel
    }

    /// Rebuild the rows when the outline changed shape, keeping the cursor on the same row.
    pub fn refresh(&mut self, scrollback: &ScrollbackState) {
        let generation = scrollback.outline_generation();
        if generation == self.generation {
            return;
        }
        let key = self.cursor_row().map(TimelineRow::key);
        self.rows = scrollback.timeline_outline();
        self.generation = generation;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        if let Some(key) = key {
            self.set_cursor_key(key);
        }
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
    pub fn follow(&mut self, key: TimelineRowKey) {
        if self.cursor_row().map(TimelineRow::key) != Some(key) {
            self.set_cursor_key(key);
        }
    }

    pub fn set_cursor(&mut self, pos: usize) -> bool {
        let pos = pos.min(self.rows.len().saturating_sub(1));
        let changed = pos != self.cursor;
        self.cursor = pos;
        changed
    }

    pub fn move_cursor(&mut self, delta: isize) -> bool {
        self.set_cursor(self.cursor.saturating_add_signed(delta))
    }

    /// Move to the nearest row above that is one level shallower.
    pub fn move_to_parent(&mut self) -> bool {
        let Some(depth) = self.cursor_row().map(|row| row.depth) else {
            return false;
        };
        match self
            .rows
            .get(..self.cursor)
            .and_then(|above| above.iter().rposition(|row| row.depth < depth))
        {
            Some(pos) => self.set_cursor(pos),
            None => false,
        }
    }

    /// Keep the cursor inside the rendered window.
    pub fn scroll_into_view(&mut self) {
        let height = self.area.height as usize;
        if height == 0 {
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(height));
    }

    /// Outline row under a screen position.
    pub fn row_at(&self, col: u16, row: u16) -> Option<usize> {
        if !self.area.contains((col, row).into()) {
            return None;
        }
        let pos = self.scroll + (row - self.area.y) as usize;
        (pos < self.rows.len()).then_some(pos)
    }
}

/// What a key does while the panel owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelinePanelInput {
    Move(isize),
    Page(isize),
    /// Left: collapse an expanded group, else step to the parent row.
    Collapse,
    /// Right: expand a collapsed group.
    Expand,
    Act(TimelineItemAction),
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
    if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
        return TimelinePanelInput::Passthrough;
    }
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => TimelinePanelInput::Move(1),
        KeyCode::Up | KeyCode::Char('k') => TimelinePanelInput::Move(-1),
        KeyCode::PageDown => TimelinePanelInput::Page(1),
        KeyCode::PageUp => TimelinePanelInput::Page(-1),
        KeyCode::Left | KeyCode::Char('h') => TimelinePanelInput::Collapse,
        KeyCode::Right | KeyCode::Char('l') => TimelinePanelInput::Expand,
        KeyCode::Enter | KeyCode::Char('f') => TimelinePanelInput::Act(TimelineItemAction::Jump),
        KeyCode::Char('e') => TimelinePanelInput::Act(TimelineItemAction::ToggleFold),
        KeyCode::Char('y') => TimelinePanelInput::Act(TimelineItemAction::CopyContent),
        KeyCode::Char('Y') => TimelinePanelInput::Act(TimelineItemAction::CopyMeta),
        KeyCode::Char('v') => TimelinePanelInput::Act(TimelineItemAction::OpenViewer),
        KeyCode::Char('R') => TimelinePanelInput::Act(TimelineItemAction::Rewind),
        KeyCode::Char('d') | KeyCode::Delete => TimelinePanelInput::Act(TimelineItemAction::Remove),
        KeyCode::Tab => TimelinePanelInput::FocusChat,
        KeyCode::Esc | KeyCode::F(6) => TimelinePanelInput::Close,
        KeyCode::F(_) => TimelinePanelInput::Passthrough,
        _ => TimelinePanelInput::Consumed,
    }
}

/// Paint the panel: one row per outline row in the window.
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
    for (pos, row) in panel
        .rows
        .iter()
        .enumerate()
        .skip(panel.scroll)
        .take(area.height as usize)
    {
        let y = area.y + (pos - panel.scroll) as u16;
        let (text, style) = row_text(row, scrollback, theme);
        let indent = "  ".repeat(usize::from(row.depth));
        let line = truncate_str(&format!("{indent}{text}"), text_w as usize);
        buf.set_line(
            text_x,
            y,
            &Line::from(Span::styled(line, style.bg(bg))),
            text_w,
        );
        if pos == panel.cursor {
            buf.set_style(Rect::new(text_x, y, text_w, 1), cursor_style);
        }
    }
}

fn row_text(row: &TimelineRow, scrollback: &ScrollbackState, theme: &Theme) -> (String, Style) {
    match row.kind {
        TimelineRowKind::Turn {
            turn_idx,
            prompt_id,
        } => {
            let preview = scrollback
                .get_by_id(prompt_id)
                .map(entry_label)
                .unwrap_or_default();
            (
                format!("\u{25BE} Turn {} {preview}", turn_idx + 1),
                Style::default()
                    .fg(theme.text_primary)
                    .add_modifier(Modifier::BOLD),
            )
        }
        TimelineRowKind::Group { first_id } => {
            let label = scrollback
                .index_of_id(first_id)
                .and_then(|idx| group_label(scrollback, idx, theme));
            let text = match label {
                Some((true, label)) => format!("\u{25BE} {label}"),
                Some((false, label)) => format!("\u{25B8} {label}"),
                None => String::new(),
            };
            (text, Style::default().fg(theme.gray_bright))
        }
        TimelineRowKind::Entry { id } => {
            let label = scrollback
                .get_by_id(id)
                .map(entry_label)
                .unwrap_or_default();
            (format!("\u{00B7} {label}"), Style::default().fg(theme.gray))
        }
    }
}

/// `(expanded, header label)` of the fold span starting at `first_idx`, worded like the chat's own header row.
fn group_label(
    scrollback: &ScrollbackState,
    first_idx: usize,
    theme: &Theme,
) -> Option<(bool, String)> {
    let span = scrollback.span_at(first_idx)?;
    let members = scrollback.entries_in_range(span.range.clone());
    let show_thinking = crate::appearance::cache::load_show_thinking_blocks();
    let text = match span.kind {
        GroupKind::VerbRun { .. } => {
            verb_group_header_label(&members, 0, members.len(), show_thinking, theme).text
        }
        GroupKind::Truncation { participants, .. } => {
            truncation_header_label(&members, 0..members.len(), None, show_thinking, theme)
                .map_or_else(|| format!("{participants} entries"), |label| label.text)
        }
    };
    Some((span.expanded, text))
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
