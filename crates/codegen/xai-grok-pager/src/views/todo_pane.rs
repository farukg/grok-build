//! Renders `TodoItem`s from `xai-grok-tools` in a `ListPane`.
//!
//! Wraps the canonical `TodoItem` type with a `ListItem` implementation that provides status-icon prefixes and styled content.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use xai_grok_shell::tools::{TodoItem, TodoStatus};

use super::list_pane::ListItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TodoStatusStyle {
    pub icon_fg: Color,
    pub text_style: Style,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TodoPaneStyle {
    pub pending: TodoStatusStyle,
    pub in_progress: TodoStatusStyle,
    pub completed: TodoStatusStyle,
    pub cancelled: TodoStatusStyle,
}

impl Default for TodoPaneStyle {
    fn default() -> Self {
        // The theme quantizes colors for terminal compatibility
        let theme = crate::theme::Theme::current();

        Self {
            pending: TodoStatusStyle {
                icon_fg: theme.text_primary,
                text_style: Style::default().fg(theme.text_primary),
            },
            in_progress: TodoStatusStyle {
                icon_fg: theme.warning,
                text_style: Style::default()
                    .fg(theme.text_primary)
                    .add_modifier(Modifier::BOLD),
            },
            completed: TodoStatusStyle {
                icon_fg: theme.accent_success,
                text_style: Style::default().fg(theme.gray_bright),
            },
            cancelled: TodoStatusStyle {
                icon_fg: theme.accent_error,
                text_style: Style::default()
                    .fg(theme.gray_bright)
                    .add_modifier(Modifier::CROSSED_OUT),
            },
        }
    }
}

/// A `TodoItem` wrapped for display in a `ListPane`.
///
/// Caches the styled `Line` for `content()` and generates a status-icon `prefix()` per frame.
#[derive(Debug, Clone)]
pub struct TodoListEntry {
    /// Unique ID (index in the todo list, or a stable ID from the model).
    pub id: u64,
    pub item: TodoItem,
    styled: Line<'static>,
    status_style: TodoStatusStyle,
}

impl TodoListEntry {
    pub fn new(id: u64, item: TodoItem, style: &TodoPaneStyle) -> Self {
        let status_style = match item.status {
            TodoStatus::Pending => style.pending,
            TodoStatus::InProgress => style.in_progress,
            TodoStatus::Completed => style.completed,
            TodoStatus::Cancelled => style.cancelled,
        };
        let styled = Line::from(Span::styled(item.content.clone(), status_style.text_style));
        Self {
            id,
            item,
            styled,
            status_style,
        }
    }

    fn prefix_line(&self) -> Line<'static> {
        Line::from(vec![
            Span::styled(self.icon(), Style::default().fg(self.status_style.icon_fg)),
            Span::raw(" "),
        ])
    }

    fn icon(&self) -> &'static str {
        match self.item.status {
            TodoStatus::Pending => "□",
            TodoStatus::InProgress => "▶",
            TodoStatus::Completed => crate::glyphs::check_mark(),
            TodoStatus::Cancelled => crate::glyphs::ballot_x(),
        }
    }
}

impl ListItem for TodoListEntry {
    fn content(&self) -> &Line<'_> {
        &self.styled
    }

    fn prefix(&self) -> Option<Line<'_>> {
        Some(self.prefix_line())
    }

    fn stable_id(&self) -> u64 {
        self.id
    }

    fn search_text(&self) -> &str {
        &self.item.content
    }
}

use crossterm::event::{KeyCode, KeyEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::StatefulWidget;

use crate::appearance::LayoutConfig;
use crate::theme::ThemeKind;

use super::list_pane::{ListPane, ListPaneConfig, ListPaneState, ListPaneStyle, WrapMode, line_display_width};
use super::overlay::OverlayState;

/// Feeds the placeholder message shown when the pane is empty.
/// Counts ALL items regardless of the `show_done` filter.
#[derive(Debug, Clone, Copy, Default)]
pub struct TodoCounts {
    pub in_progress: usize,
    pub pending: usize,
    pub completed: usize,
    pub cancelled: usize,
}

impl TodoCounts {
    pub fn total(&self) -> usize {
        self.in_progress + self.pending + self.completed + self.cancelled
    }
}

fn empty_placeholder_message(todos_empty: bool, counts: TodoCounts) -> String {
    if todos_empty {
        return "No todo items.".into();
    }
    match (counts.completed, counts.cancelled) {
        (_, 0) => "All done.".into(),
        (0, c) => format!("{c} cancelled."),
        (d, c) => format!("{d} done. {c} cancelled."),
    }
}

/// Absolute maximum height (in lines) the todo pane will request.
const MAX_TODO_HEIGHT: u16 = 10;

/// Maximum fraction of total view height the todo pane may occupy.
const MAX_TODO_FRACTION: f32 = 0.15;

/// Owns the raw `TodoItem` data, the filtered `TodoListEntry` cache, `ListPaneState`, and style config.
/// `AgentView` holds a single `TodoPane` and delegates input and render to it.
pub struct TodoPane {
    /// Raw todo items from ACP Plan updates.
    todos: Vec<TodoItem>,
    /// Filtered and styled entries for `ListPane` rendering.
    /// Rebuilt only when data, filtering or status styles change.
    entries: Vec<TodoListEntry>,
    counts: TodoCounts,
    sidebar_cache: std::cell::RefCell<Option<SidebarCache>>,
    #[cfg(test)]
    preparation_rebuilds: usize,
    #[cfg(test)]
    sidebar_materializations: std::cell::Cell<usize>,
    #[cfg(test)]
    sidebar_projections: std::cell::Cell<usize>,
    /// List pane state (scroll, selection, search, layout cache).
    pub list_state: ListPaneState,
    /// Per-status visual style for icons and text.
    style: TodoPaneStyle,
    /// Visual style for the `ListPane` widget (selection bg, etc.).
    list_style: ListPaneStyle,
    /// Whether to show completed and cancelled items.
    show_done: bool,
    /// Shared visibility and focus state.
    pub overlay: OverlayState,
    /// Last theme kind seen; used to detect theme switches and restyle.
    last_theme: ThemeKind,
}

impl Default for TodoPane {
    fn default() -> Self {
        Self::new()
    }
}

impl TodoPane {
    pub fn new() -> Self {
        let config = ListPaneConfig {
            follow_enabled: false,
            wrap_toggle_enabled: false,
            search_enabled: true,
            copy_enabled: true,
            show_selection_when_unfocused: false,
            visual_select_enabled: false,
            filter_enabled: true,
            goto_line_enabled: false,
        };
        let mut list_state = ListPaneState::new_with_config(WrapMode::NoWrap, false, config);
        list_state.set_clipboard_provider(Box::new(crate::clipboard::SystemClipboard));
        Self {
            todos: Vec::new(),
            entries: Vec::new(),
            counts: TodoCounts::default(),
            sidebar_cache: std::cell::RefCell::new(None),
            #[cfg(test)]
            preparation_rebuilds: 0,
            #[cfg(test)]
            sidebar_materializations: std::cell::Cell::new(0),
            #[cfg(test)]
            sidebar_projections: std::cell::Cell::new(0),
            list_state,
            style: TodoPaneStyle::default(),
            list_style: ListPaneStyle::default(),
            show_done: crate::app::panel_settings::CompletedItemsPane::Todos.default_visibility(),
            overlay: OverlayState::hidden(),
            last_theme: crate::theme::Theme::current_kind(),
        }
    }

    pub fn todos(&self) -> &[TodoItem] {
        &self.todos
    }

    /// Replace all todo items (called from ACP Plan handler).
    ///
    /// Does NOT auto-show the todo pane. Users toggle it with Ctrl-T.
    pub fn update_todos(&mut self, items: Vec<TodoItem>) {
        let changed = self.todos.len() != items.len()
            || self.todos.iter().zip(&items).any(|(old, new)| {
                old.content != new.content || old.status != new.status
            });
        self.counts = Self::compute_counts(&items);
        self.todos = items;
        if changed {
            self.rebuild_entries();
        }
    }

    fn compute_counts(items: &[TodoItem]) -> TodoCounts {
        let mut c = TodoCounts::default();
        for item in items {
            match item.status {
                TodoStatus::InProgress => c.in_progress += 1,
                TodoStatus::Pending => c.pending += 1,
                TodoStatus::Completed => c.completed += 1,
                TodoStatus::Cancelled => c.cancelled += 1,
            }
        }
        c
    }

    /// Current status counts (across ALL items, ignoring `show_done`).
    pub fn counts(&self) -> TodoCounts {
        self.counts
    }

    /// Visible when the overlay is shown.
    /// When empty, the pane shows a placeholder message instead of the list.
    pub fn is_visible(&self) -> bool {
        self.overlay.visible
    }

    /// Called after an overlay state change hides the pane.
    /// Closes the input bar if it's open mid-typing, but keeps any accepted search or filter across hide and show.
    pub fn on_state_change(&mut self) {
        if !self.overlay.visible {
            self.list_state.close_input_bar();
        }
    }

    /// Whether completed and cancelled items are shown.
    pub fn show_done(&self) -> bool {
        self.show_done
    }

    /// Toggle visibility of completed and cancelled items.
    pub fn set_show_done(&mut self, show: bool) {
        if self.show_done != show {
            self.show_done = show;
            self.rebuild_entries();
        }
    }

    pub fn toggle_show_done(&mut self) {
        self.set_show_done(!self.show_done);
    }

    /// Desired height in lines for layout computation. Returns 0 when hidden (overlay not visible or no
    /// items). When visible but empty, returns 2 (for placeholder message). Otherwise: `min(10, 15% of
    /// view_height)` but at least 1.
    pub fn desired_height(&self, view_height: u16) -> u16 {
        if !self.overlay.visible {
            return 0;
        }
        let count = self.visible_count();
        if count == 0 {
            return 1;
        }
        let fraction_cap = (view_height as f32 * MAX_TODO_FRACTION).floor() as u16;
        let max = MAX_TODO_HEIGHT.min(fraction_cap).max(1);
        (count as u16).min(max).max(1)
    }

    /// Number of items that pass the `show_done` filter.
    fn visible_count(&self) -> usize {
        self.entries.len()
    }

    /// Rebuild `entries` from `todos`, filtered by `show_done`. Items preserve their original order
    /// from the agent (no reordering by status). IDs are based on the original index in `todos` so that
    /// `ListPaneState` can maintain selection across rebuilds.
    fn rebuild_entries(&mut self) {
        #[cfg(test)]
        {
            self.preparation_rebuilds += 1;
        }
        *self.sidebar_cache.get_mut() = None;
        self.entries.clear();
        for (idx, item) in self.todos.iter().enumerate() {
            if !self.show_done
                && matches!(item.status, TodoStatus::Completed | TodoStatus::Cancelled)
            {
                continue;
            }
            self.entries
                .push(TodoListEntry::new(idx as u64, item.clone(), &self.style));
        }
        self.list_state.invalidate_layout();
        let matcher = self.list_state.matcher().cloned();
        self.list_state.set_matcher(matcher);
    }

    pub(crate) fn sidebar_row_count(&self, text_width: u16) -> usize {
        self.sidebar_projection(text_width).row_count()
    }

    #[cfg(test)]
    pub(crate) fn preparation_counts(&self) -> (usize, usize) {
        (self.preparation_rebuilds, self.sidebar_materializations.get())
    }

    #[cfg(test)]
    pub(crate) fn sidebar_projection_count(&self) -> usize {
        self.sidebar_projections.get()
    }

    pub(crate) fn refresh_entries(&mut self) {
        let current_theme = crate::theme::Theme::current_kind();
        let style = TodoPaneStyle::default();
        self.list_style = ListPaneStyle::default();
        if current_theme != self.last_theme || style != self.style {
            self.last_theme = current_theme;
            self.style = style;
            self.rebuild_entries();
        }
    }

    /// Handle a key event when the todo pane is focused.
    ///
    /// Returns `true` if the event was consumed.
    pub fn handle_key(&mut self, key: &KeyEvent) -> bool {
        // 'h' toggles show_done (only when not typing in the search or filter bar)
        if key.code == KeyCode::Char('h') && self.list_state.input_mode().is_none() {
            self.toggle_show_done();
            return true;
        }
        // Don't route to ListPaneState when empty.
        if self.entries.is_empty() {
            return false;
        }
        self.list_state.handle_key_event(key, &self.entries)
    }

    /// The pane's real End boundary: a plain `End` would not move the selection.
    pub fn at_end_boundary(&self) -> bool {
        self.list_state.at_last_selectable(&self.entries)
    }

    pub fn handle_paste(&mut self, text: &str) -> bool {
        self.list_state.handle_paste(text, &self.entries)
    }

    /// Handle a mouse scroll event over the todo pane area. Caps scroll speed for small viewports. The
    /// app-level scroll accumulator can produce deltas of 3-5 lines, which would jump past most items
    /// in a 4-row pane.
    pub fn handle_scroll(&mut self, lines: i32, col: u16, row: u16) {
        let max = match self.list_state.viewport_height() {
            0..=5 => 1,
            6..=10 => 2,
            _ => lines.unsigned_abs() as i32,
        };
        let capped = lines.signum() * lines.abs().min(max);
        self.list_state
            .handle_scroll_event(capped, col, row, &self.entries);
    }

    /// Handle a mouse click or drag event over the todo pane area.
    ///
    /// Returns `true` if the event was consumed.
    pub fn handle_mouse(&mut self, kind: MouseEventKind, col: u16, row: u16, area: Rect) -> bool {
        self.list_state
            .handle_mouse_event(kind, col, row, area, &self.entries)
    }

    /// Compute the inner content area with horizontal padding matching the scrollback's `HorizontalLayout`.
    /// The left pad is accent + block_pad_left; the right pad is block_pad_right.
    fn content_area(area: Rect, layout_cfg: &LayoutConfig) -> Rect {
        use crate::scrollback::layout::HorizontalLayout;
        let pad_left = HorizontalLayout::ACCENT + layout_cfg.block_pad_left;
        let pad_right = layout_cfg.block_pad_right;
        Rect {
            x: area.x + pad_left,
            y: area.y,
            width: area.width.saturating_sub(pad_left + pad_right),
            height: area.height,
        }
    }

    /// Render the cached entries in a padded area matching the scrollback's horizontal layout.
    pub fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focused: bool,
        layout_cfg: &LayoutConfig,
    ) {
        self.refresh_entries();
        let inner = Self::content_area(area, layout_cfg);
        if self.entries.is_empty() {
            if inner.height > 0 && inner.width > 0 {
                let msg = empty_placeholder_message(self.todos.is_empty(), self.counts());
                let theme = crate::theme::Theme::current();
                let span = ratatui::text::Span::styled(
                    msg,
                    ratatui::style::Style::default().fg(theme.gray_bright),
                );
                buf.set_span(inner.x, inner.y, &span, inner.width);
            }
            return;
        }
        self.list_state
            .prepare_layout(&self.entries, inner.width, inner.height);
        ListPane::new(&self.entries)
            .focused(focused)
            .style(self.list_style)
            .render(inner, buf, &mut self.list_state);
    }
}

/// The most rows the F7 Todos section may take, so Context and Message display stay reachable.
/// The pointer to the todo pane counts against the budget whenever anything stays hidden.
const MAX_SIDEBAR_ROWS: usize = 6;

/// The F7 Todos section as painted lines: whole entries while they fit, the clipped tail of the
/// entry the budget cuts into, and the pointer to the todo pane for everything that stays hidden.
pub(crate) struct SidebarProjection {
    lines: Vec<Line<'static>>,
}

struct SidebarCache {
    text_width: u16,
    marker_style: Style,
    projection: std::sync::Arc<SidebarProjection>,
}

impl SidebarProjection {
    pub(crate) fn row_count(&self) -> usize {
        self.lines.len()
    }

    fn row(&self, index: usize) -> Option<super::sidebar::SidebarRow<'static>> {
        Some(super::sidebar::SidebarRow {
            left: self.lines.get(index)?.clone(),
            right: None,
            detail: &[],
        })
    }
}

impl TodoPane {
    /// One entry as wrapped sidebar lines: the status prefix, continuation indented under the text.
    fn wrapped_lines(&self, entry: &TodoListEntry, text_width: u16) -> Vec<Line<'static>> {
        if text_width == 0 {
            let mut whole = entry.prefix_line();
            whole.spans.extend(entry.styled.spans.iter().cloned());
            return vec![whole];
        }
        let prefix = entry.prefix_line();
        let indent = Line::from(" ".repeat(line_display_width(&prefix)));
        crate::render::wrapping::word_wrap_line_prefix::<{ MAX_SIDEBAR_ROWS + 1 }>(
            &entry.styled,
            text_width as usize,
            prefix,
            indent,
        )
    }

    /// The Todos section at one text width, bounded to [`MAX_SIDEBAR_ROWS`]: whole entries while
    /// they fit, the first entry cut when it alone exceeds the budget, and a pointer that counts
    /// the wholly hidden entries and says when the last shown one continues.
    pub(crate) fn sidebar_projection(&self, text_width: u16) -> std::sync::Arc<SidebarProjection> {
        let marker_style = Style::default().fg(crate::theme::Theme::current().gray_bright);
        if let Some(cache) = self.sidebar_cache.borrow().as_ref()
            && cache.text_width == text_width
            && cache.marker_style == marker_style
        {
            return cache.projection.clone();
        }
        let projection = std::sync::Arc::new(self.build_sidebar_projection(text_width));
        *self.sidebar_cache.borrow_mut() = Some(SidebarCache {
            text_width,
            marker_style,
            projection: projection.clone(),
        });
        projection
    }

    fn build_sidebar_projection(&self, text_width: u16) -> SidebarProjection {
        #[cfg(test)]
        self.sidebar_projections.set(self.sidebar_projections.get() + 1);
        if self.entries.is_empty() {
            return SidebarProjection {
                lines: vec![Line::from(empty_placeholder_message(
                    self.todos.is_empty(),
                    self.counts(),
                ))],
            };
        }
        let mut wrapped: Vec<Vec<Line<'static>>> = Vec::new();
        let mut rows = 0;
        for entry in &self.entries {
            let entry_lines = self.wrapped_lines(entry, text_width);
            rows += entry_lines.len();
            let capped = rows > MAX_SIDEBAR_ROWS;
            wrapped.push(entry_lines);
            if capped {
                return SidebarProjection {
                    lines: self.capped_lines(wrapped, text_width),
                };
            }
        }
        SidebarProjection {
            lines: wrapped.into_iter().flatten().collect(),
        }
    }

    /// The bounded tail once the entries exceed the budget: entries share `MAX_SIDEBAR_ROWS - 1`
    /// rows because the pointer takes the last one.
    fn capped_lines(
        &self,
        wrapped: Vec<Vec<Line<'static>>>,
        text_width: u16,
    ) -> Vec<Line<'static>> {
        let entry_budget = MAX_SIDEBAR_ROWS - 1;
        let mut lines: Vec<Line<'static>> = Vec::with_capacity(MAX_SIDEBAR_ROWS);
        let mut whole = 0;
        for entry_lines in &wrapped {
            if lines.len() + entry_lines.len() > entry_budget {
                break;
            }
            lines.extend(entry_lines.iter().cloned());
            whole += 1;
        }
        // The entry the budget cuts into starts inside it: show its first rows and mark the cut.
        let mut continued = false;
        if lines.len() < entry_budget && let Some(entry_lines) = wrapped.get(whole) {
            let take = entry_budget - lines.len();
            lines.extend(entry_lines.iter().take(take).cloned());
            continued = take < entry_lines.len();
            if continued && let Some(last) = lines.last_mut() {
                let marker = Style::default().fg(crate::theme::Theme::current().gray_bright);
                // Two columns stay free so the marker survives the row renderer's width cut.
                let marker_budget = text_width.saturating_sub(2).max(1) as usize;
                *last = crate::render::line_utils::truncate_line(std::mem::take(last), marker_budget);
                last.spans.push(Span::styled(" …", marker));
            }
        }
        let hidden = self.entries.len() - whole - usize::from(continued);
        let pointer = match (continued, hidden) {
            (true, 0) => "… continued · Ctrl+T".to_owned(),
            (true, more) => format!("… {more} more, 1 continued · Ctrl+T"),
            (false, more) => format!("… {more} more · Ctrl+T"),
        };
        lines.push(Line::from(Span::styled(
            pointer,
            Style::default().fg(crate::theme::Theme::current().gray_bright),
        )));
        lines
    }

    /// A row of the F7 Todos section, from its cached projection: a wrapped entry line, the clipped
    /// tail of the last entry, or the pointer to the rest.
    pub(crate) fn sidebar_row(
        &self,
        index: usize,
        text_width: u16,
    ) -> Option<super::sidebar::SidebarRow<'static>> {
        self.sidebar_row_of(&self.sidebar_projection(text_width), index)
    }

    /// One painted row served from a projection the caller holds, so a frame counts and paints
    /// the same one.
    pub(crate) fn sidebar_row_of(
        &self,
        projection: &SidebarProjection,
        index: usize,
    ) -> Option<super::sidebar::SidebarRow<'static>> {
        #[cfg(test)]
        self.sidebar_materializations.set(self.sidebar_materializations.get() + 1);
        projection.row(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(completed: usize, cancelled: usize) -> TodoCounts {
        TodoCounts {
            completed,
            cancelled,
            ..TodoCounts::default()
        }
    }

    #[test]
    fn empty_todos_message() {
        assert_eq!(
            empty_placeholder_message(true, TodoCounts::default()),
            "No todo items."
        );
    }

    #[test]
    fn all_completed_is_all_done() {
        assert_eq!(empty_placeholder_message(false, counts(3, 0)), "All done.");
    }

    #[test]
    fn mixed_done_and_cancelled_summarizes_counts() {
        assert_eq!(
            empty_placeholder_message(false, counts(5, 1)),
            "5 done. 1 cancelled."
        );
    }

    #[test]
    fn only_cancelled() {
        assert_eq!(
            empty_placeholder_message(false, counts(0, 2)),
            "2 cancelled."
        );
    }

    fn flat(row: &super::super::sidebar::SidebarRow<'_>) -> String {
        row.left.spans.iter().map(|span| span.content.clone().into_owned()).collect()
    }

    fn visible_width(line: &Line<'_>) -> usize {
        use unicode_width::UnicodeWidthStr;
        line.spans.iter().map(|span| span.content.width()).sum()
    }

    fn pane_with(items: &[(&str, TodoStatus)]) -> TodoPane {
        let mut pane = TodoPane::new();
        pane.update_todos(
            items
                .iter()
                .map(|&(content, status)| TodoItem {
                    content: content.to_owned(),
                    status,
                    priority: Default::default(),
                    meta: None,
                })
                .collect(),
        );
        pane
    }

    #[test]
    fn sidebar_cache_tracks_content_status_filter_width_and_style() {
        let _theme = crate::theme::cache::pin_theme();
        crate::theme::cache::set(crate::theme::ThemeKind::GrokNight);
        let mut pane = pane_with(&[("Before", TodoStatus::Pending)]);
        let width = 24;
        assert_eq!(flat(&pane.sidebar_row(0, width).unwrap()), "□ Before");
        let prepared = pane.sidebar_projection_count();
        for _ in 0..8 {
            assert_eq!(pane.sidebar_row_count(width), 1);
            assert_eq!(flat(&pane.sidebar_row(0, width).unwrap()), "□ Before");
        }
        assert_eq!(pane.sidebar_projection_count(), prepared);
        pane.update_todos(vec![TodoItem {
            content: "Before".into(),
            status: TodoStatus::Pending,
            priority: Default::default(),
            meta: None,
        }]);
        assert_eq!(flat(&pane.sidebar_row(0, width).unwrap()), "□ Before");
        assert_eq!(pane.sidebar_projection_count(), prepared, "identical data is not a new projection");
        pane.update_todos(vec![TodoItem {
            content: "Before".into(),
            status: TodoStatus::InProgress,
            priority: Default::default(),
            meta: None,
        }]);
        assert_eq!(flat(&pane.sidebar_row(0, width).unwrap()), "▶ Before");
        assert_eq!(pane.sidebar_projection_count(), prepared + 1);
        pane.update_todos(vec![TodoItem {
            content: "After".into(),
            status: TodoStatus::InProgress,
            priority: Default::default(),
            meta: None,
        }]);
        assert_eq!(flat(&pane.sidebar_row(0, width).unwrap()), "▶ After");
        assert_eq!(pane.sidebar_projection_count(), prepared + 2);
        pane.set_show_done(false);
        pane.update_todos(vec![TodoItem {
            content: "Finished".into(),
            status: TodoStatus::Completed,
            priority: Default::default(),
            meta: None,
        }]);
        assert_eq!(flat(&pane.sidebar_row(0, width).unwrap()), "All done.");
        let filtered = pane.sidebar_projection_count();
        pane.set_show_done(true);
        assert!(flat(&pane.sidebar_row(0, width).unwrap()).contains("Finished"));
        assert_eq!(pane.sidebar_projection_count(), filtered + 1);
        let wide = pane.sidebar_projection_count();
        assert!(pane.sidebar_row_count(7) > 1);
        assert_eq!(pane.sidebar_projection_count(), wide + 1);
        assert!(pane.sidebar_row_count(7) > 1);
        assert_eq!(pane.sidebar_projection_count(), wide + 1);
        crate::theme::cache::set(crate::theme::ThemeKind::GrokDay);
        pane.refresh_entries();
        let row = pane.sidebar_row(0, 7).unwrap();
        assert_eq!(row.left.spans[0].style.fg, Some(crate::theme::Theme::current().accent_success));
        assert_eq!(pane.sidebar_projection_count(), wide + 2);
    }

    #[test]
    fn sidebar_rows_wrap_long_labels_within_the_text_width() {
        let _theme = crate::theme::cache::pin_theme();
        let label = "Refactor the context sidebar footer so every section explains its own keys";
        let pane = pane_with(&[(label, TodoStatus::InProgress)]);
        let width = 30;
        assert!(pane.sidebar_row_count(width) > 1, "a long label wraps");
        for index in 0..pane.sidebar_row_count(width) {
            let row = pane.sidebar_row(index, width).expect("row exists");
            let text = flat(&row);
            assert!(visible_width(&row.left) <= width, "{index}: {text}");
            if index == 0 {
                assert!(text.starts_with("▶ "), "{text}");
            } else {
                assert!(text.starts_with("  "), "continuation is indented under the text: {text}");
            }
        }
        let joined = (0..pane.sidebar_row_count(width))
            .filter_map(|index| pane.sidebar_row(index, width).map(|row| flat(&row)))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("Refactor the context sidebar"), "{joined}");
        assert!(joined.contains("explains its own keys"), "{joined}");
    }

    #[test]
    fn sidebar_rows_measure_wide_characters_by_display_width() {
        let _theme = crate::theme::cache::pin_theme();
        let pane = pane_with(&[("日本語のタスク名が長い場合でも折り返しが正しく機能することを確認する", TodoStatus::Pending)]);
        let width = 20;
        assert!(pane.sidebar_row_count(width) > 1);
        for index in 0..pane.sidebar_row_count(width) {
            let row = pane.sidebar_row(index, width).expect("row exists");
            assert!(visible_width(&row.left) <= width, "{}", flat(&row));
        }
    }

    #[test]
    fn sidebar_budget_holds_for_a_huge_first_entry_and_marks_the_cut() {
        let _theme = crate::theme::cache::pin_theme();
        let label = "Check every entry of the release checklist end to end ".repeat(4);
        let pane = pane_with(&[(label.as_str(), TodoStatus::InProgress)]);
        let width = 17;
        let count = pane.sidebar_row_count(width);
        assert_eq!(count, MAX_SIDEBAR_ROWS, "a huge first entry gets the budget like any other");
        let first = flat(&pane.sidebar_row(0, width).expect("first row"));
        assert!(first.starts_with("▶ "), "{first}");
        let cut = flat(&pane.sidebar_row(count - 2, width).expect("cut entry tail"));
        assert!(cut.ends_with(" …"), "the clipped entry marks the cut on its last shown line: {cut}");
        let pointer = flat(&pane.sidebar_row(count - 1, width).expect("pointer"));
        assert_eq!(pointer, "… continued · Ctrl+T");
        assert!(pane.sidebar_row(count, width).is_none(), "nothing paints beyond the budget");
    }

    #[test]
    fn sidebar_budget_holds_for_a_huge_wide_character_first_entry() {
        let _theme = crate::theme::cache::pin_theme();
        let label = "非常に長いタスク名でも予算を守り最終行に省略記号を出すことを確認するためのテスト用ラベル";
        let pane = pane_with(&[(label, TodoStatus::Pending)]);
        let width = 17;
        let count = pane.sidebar_row_count(width);
        assert_eq!(count, MAX_SIDEBAR_ROWS);
        let cut = flat(&pane.sidebar_row(count - 2, width).expect("cut entry tail"));
        assert!(cut.ends_with(" …"), "the clipped entry marks the cut: {cut}");
        assert_eq!(
            flat(&pane.sidebar_row(count - 1, width).expect("pointer")),
            "… continued · Ctrl+T"
        );
    }

    #[test]
    fn sidebar_pointer_counts_wholly_hidden_entries_and_distinguishes_the_cut_one() {
        let _theme = crate::theme::cache::pin_theme();

        fn short_items(count: usize) -> Vec<TodoItem> {
            (0..count)
                .map(|index| TodoItem {
                    content: format!("Task {index}"),
                    status: TodoStatus::Pending,
                    priority: Default::default(),
                    meta: None,
                })
                .collect()
        }

        let mut pane = TodoPane::new();
        pane.update_todos(short_items(6));
        let width = 30;
        assert_eq!(pane.sidebar_row_count(width), MAX_SIDEBAR_ROWS);
        let last_entry = flat(&pane.sidebar_row(MAX_SIDEBAR_ROWS - 1, width).expect("sixth entry"));
        assert!(last_entry.contains("Task 5"), "{last_entry}");
        assert!(!last_entry.contains("more"), "six fitting entries claim nothing hidden");

        pane.update_todos(short_items(7));
        let count = pane.sidebar_row_count(width);
        assert_eq!(count, MAX_SIDEBAR_ROWS);
        assert_eq!(
            flat(&pane.sidebar_row(count - 1, width).expect("pointer")),
            "… 2 more · Ctrl+T",
            "only the wholly hidden entries are counted"
        );

        let mut items = short_items(2);
        items.push(TodoItem {
            content: "Trim the wrapped sidebar projection so a cut entry keeps an honest pointer and stays inside the row budget end to end".to_owned(),
            status: TodoStatus::InProgress,
            priority: Default::default(),
            meta: None,
        });
        items.extend(short_items(2));
        pane.update_todos(items);
        let count = pane.sidebar_row_count(width);
        assert_eq!(count, MAX_SIDEBAR_ROWS);
        let cut = flat(&pane.sidebar_row(count - 2, width).expect("cut entry tail"));
        assert!(cut.ends_with(" …"), "{cut}");
        assert_eq!(
            flat(&pane.sidebar_row(count - 1, width).expect("pointer")),
            "… 2 more, 1 continued · Ctrl+T",
            "the partially shown entry is distinguished from the wholly hidden ones"
        );
    }

    #[test]
    fn sidebar_rows_stay_under_the_budget_and_point_to_the_pane_for_the_rest() {
        let _theme = crate::theme::cache::pin_theme();
        let items: Vec<_> = (0..9)
            .map(|index| (format!("Task {index} with a label"), if index % 2 == 0 { TodoStatus::Pending } else { TodoStatus::InProgress }))
            .collect();
        let references: Vec<(&str, TodoStatus)> = items.iter().map(|(content, status)| (content.as_str(), *status)).collect();
        let pane = pane_with(&references);
        let width = 60;
        let count = pane.sidebar_row_count(width);
        assert!(count <= MAX_SIDEBAR_ROWS, "the pointer shares the budget: {count}");
        let last = flat(&pane.sidebar_row(count - 1, width).expect("last row"));
        assert!(last.contains("more"), "{last}");
        assert!(last.contains("Ctrl+T"), "{last}");
        assert!(pane.sidebar_row(count, width).is_none(), "no rows beyond the count");
    }

    #[test]
    fn sidebar_rows_skip_completed_entries_by_the_pane_filter_and_keep_the_placeholder() {
        let _theme = crate::theme::cache::pin_theme();
        let mut pane = pane_with(&[
            ("Pending work", TodoStatus::Pending),
            ("Finished work", TodoStatus::Completed),
        ]);
        pane.set_show_done(false);
        let width = 40;
        assert_eq!(pane.sidebar_row_count(width), 1);
        assert_eq!(flat(&pane.sidebar_row(0, width).expect("row")), "□ Pending work");
        assert!(pane.sidebar_row(1, width).is_none());

        pane.update_todos(Vec::new());
        assert_eq!(flat(&pane.sidebar_row(0, width).expect("row")), "No todo items.");
        pane.update_todos(vec![TodoItem {
            content: "Only finished".to_owned(),
            status: TodoStatus::Completed,
            priority: Default::default(),
            meta: None,
        }]);
        assert_eq!(flat(&pane.sidebar_row(0, width).expect("row")), "All done.");
    }
}
