//! Expanded-timeline wiring for [`AgentView`]: open/close, key and mouse routing, scroll sync, and item/group actions.
//! Every action reuses an existing scrollback operation or `Action` on the row's entry; the panel only chooses the target.

use super::{AgentPane, AgentView};
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::scrollback::state::{TimelineRow, TimelineRowKind};
use crate::views::timeline_panel::{
    PanelFocus, TimelineItemAction, TimelineMode, TimelinePanelInput, TimelinePanelState,
    timeline_panel_key,
};
use crossterm::event::{Event, MouseButton, MouseEventKind};
use ratatui::layout::Rect;

impl AgentView {
    /// F6 / palette: open the panel on the viewport, focus an unfocused panel, or fold a focused one back to the rail.
    pub(crate) fn toggle_timeline_panel(&mut self) -> InputOutcome {
        if self.is_minimal_mode() {
            return InputOutcome::Unchanged;
        }
        let focus = self.timeline_mode.panel().map(|panel| panel.focus);
        match focus {
            None => {
                let at = self.scrollback.timeline_follow_index();
                self.open_timeline_panel_at(at)
            }
            Some(PanelFocus::Chat) => {
                if let Some(panel) = self.timeline_mode.panel_mut() {
                    panel.focus = PanelFocus::Panel;
                }
                InputOutcome::Changed
            }
            Some(PanelFocus::Panel) => {
                self.timeline_mode = TimelineMode::Rail;
                InputOutcome::Changed
            }
        }
    }

    /// Open (or re-point) the panel with the cursor on the entry at `at`.
    pub(crate) fn open_timeline_panel_at(&mut self, at: Option<usize>) -> InputOutcome {
        if self.is_minimal_mode() {
            return InputOutcome::Unchanged;
        }
        self.timeline_mode =
            TimelineMode::Expanded(TimelinePanelState::open(&self.scrollback, at));
        InputOutcome::Changed
    }

    /// Per-frame panel geometry and the chat → panel sync.
    /// The outline rebuilds only on structural change; following is a no-op unless the tracked entry moved.
    /// Expects the scrollback layout prepared for this frame.
    pub(super) fn sync_timeline_panel_frame(
        &mut self,
        scrollback_area: Rect,
        column_x: u16,
        column_width: u16,
    ) {
        let Some(panel) = self.timeline_mode.panel_mut() else {
            return;
        };
        if column_width == 0 {
            panel.area = Rect::default();
            return;
        }
        panel.area = Rect {
            x: column_x,
            y: scrollback_area.y,
            width: column_width,
            height: scrollback_area.height,
        };
        panel.refresh(&self.scrollback);
        if panel.focus == PanelFocus::Chat
            && let Some(key) = self
                .scrollback
                .timeline_follow_index()
                .and_then(|idx| self.scrollback.timeline_row_key(idx))
        {
            panel.follow(key);
        }
        panel.scroll_into_view();
    }

    /// Route input while the panel is focused. `None` lets the event continue through the normal chain.
    pub(super) fn route_timeline_panel_input(&mut self, ev: &Event) -> Option<InputOutcome> {
        if !self.no_input_overlay_pending() {
            return None;
        }
        let panel = self.timeline_mode.panel_mut()?;
        match ev {
            Event::Key(key) if panel.focus == PanelFocus::Panel => {
                self.handle_timeline_panel_input(timeline_panel_key(key))
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                match panel.row_at(mouse.column, mouse.row) {
                    Some(pos) => {
                        panel.focus = PanelFocus::Panel;
                        panel.set_cursor(pos);
                        self.sync_timeline_cursor_to_chat();
                        Some(InputOutcome::Changed)
                    }
                    None => {
                        panel.focus = PanelFocus::Chat;
                        None
                    }
                }
            }
            _ => None,
        }
    }

    fn handle_timeline_panel_input(&mut self, input: TimelinePanelInput) -> Option<InputOutcome> {
        let panel = self.timeline_mode.panel_mut()?;
        let outcome = match input {
            TimelinePanelInput::Move(delta) => {
                if panel.move_cursor(delta) {
                    self.sync_timeline_cursor_to_chat();
                }
                InputOutcome::Changed
            }
            TimelinePanelInput::Page(sign) => {
                let page = (panel.area.height.max(2) - 1) as isize;
                if panel.move_cursor(sign * page) {
                    self.sync_timeline_cursor_to_chat();
                }
                InputOutcome::Changed
            }
            TimelinePanelInput::Collapse => {
                if !self.set_timeline_group_expanded(false) {
                    let moved = self
                        .timeline_mode
                        .panel_mut()
                        .is_some_and(TimelinePanelState::move_to_parent);
                    if moved {
                        self.sync_timeline_cursor_to_chat();
                    }
                }
                InputOutcome::Changed
            }
            TimelinePanelInput::Expand => {
                self.set_timeline_group_expanded(true);
                InputOutcome::Changed
            }
            TimelinePanelInput::Act(action) => self.run_timeline_item_action(action),
            TimelinePanelInput::FocusChat => {
                panel.focus = PanelFocus::Chat;
                InputOutcome::Changed
            }
            TimelinePanelInput::Close => {
                self.timeline_mode = TimelineMode::Rail;
                InputOutcome::Changed
            }
            TimelinePanelInput::Consumed => InputOutcome::Changed,
            TimelinePanelInput::Passthrough => return None,
        };
        Some(outcome)
    }

    /// Wheel over the panel moves its cursor; wheel over the chat hands the selection back to the chat.
    /// Returns whether the panel consumed the wheel.
    pub(super) fn scroll_timeline_panel(&mut self, lines: i32, col: u16, row: u16) -> bool {
        let Some(panel) = self.timeline_mode.panel_mut() else {
            return false;
        };
        if panel.area.contains((col, row).into()) {
            panel.focus = PanelFocus::Panel;
            if panel.move_cursor(lines.signum() as isize) {
                self.sync_timeline_cursor_to_chat();
            }
            return true;
        }
        panel.focus = PanelFocus::Chat;
        false
    }

    fn timeline_cursor(&self) -> Option<(TimelineRow, usize)> {
        let row = *self.timeline_mode.panel()?.cursor_row()?;
        let idx = self.scrollback.index_of_id(row.entry_id())?;
        Some((row, idx))
    }

    /// Panel → chat: select the cursor's entry (or the fold row showing it) and bring it to the viewport top.
    fn sync_timeline_cursor_to_chat(&mut self) {
        let Some((_, idx)) = self.timeline_cursor() else {
            return;
        };
        let anchor = self.scrollback.timeline_anchor(idx);
        self.scrollback.set_selected(Some(anchor));
        self.scrollback.scroll_to_entry_top(anchor);
    }

    /// Expand or collapse the cursor's group in panel and chat at once (the chat's own group toggle).
    /// `false` when the cursor is not on a group row or the group is already in that state.
    fn set_timeline_group_expanded(&mut self, expand: bool) -> bool {
        let Some((row, first)) = self.timeline_cursor() else {
            return false;
        };
        match row.kind {
            TimelineRowKind::Group { .. } => {}
            TimelineRowKind::Turn { .. } | TimelineRowKind::Entry { .. } => return false,
        }
        let Some(expanded) = self.scrollback.span_at(first).map(|span| span.expanded) else {
            return false;
        };
        if expanded == expand {
            return false;
        }
        if expand {
            let header = self.scrollback.group_header_index(first).unwrap_or(first);
            self.scrollback.set_selected(Some(header));
            self.scrollback.toggle_group_expansion()
        } else {
            self.scrollback.set_selected(Some(first));
            self.scrollback.collapse_group_if_expanded()
        }
    }

    /// Unfold a collapsed group hiding the entry, so entry-level actions see its own content.
    fn reveal_timeline_entry(&mut self, idx: usize) {
        if self.scrollback.entry_content_hidden_by_group(idx)
            && self.scrollback.span_at(idx).is_some_and(|span| !span.expanded)
            && let Some(header) = self.scrollback.group_header_index(idx)
        {
            self.scrollback.set_selected(Some(header));
            self.scrollback.toggle_group_expansion();
        }
        self.scrollback.set_selected(Some(idx));
    }

    /// Apply `action` to the cursor row; a group row acts on the group as a whole.
    fn run_timeline_item_action(&mut self, action: TimelineItemAction) -> InputOutcome {
        let Some((row, idx)) = self.timeline_cursor() else {
            return InputOutcome::Unchanged;
        };
        let target = match row.kind {
            TimelineRowKind::Group { .. } => ActionTarget::Group,
            TimelineRowKind::Turn { .. } | TimelineRowKind::Entry { .. } => ActionTarget::Entry,
        };
        match (action, target) {
            (TimelineItemAction::Jump, ActionTarget::Entry | ActionTarget::Group) => {
                let anchor = self.scrollback.timeline_anchor(idx);
                self.scrollback.set_selected(Some(anchor));
                self.scrollback.scroll_to_entry_top(anchor);
                if let Some(panel) = self.timeline_mode.panel_mut() {
                    panel.focus = PanelFocus::Chat;
                }
                self.set_active_pane(AgentPane::Scrollback, false);
                InputOutcome::Changed
            }
            (TimelineItemAction::ToggleFold, ActionTarget::Group) => {
                let expanded = self
                    .scrollback
                    .span_at(idx)
                    .is_some_and(|span| span.expanded);
                self.set_timeline_group_expanded(!expanded);
                InputOutcome::Changed
            }
            // The entry's own fold; `Action::ToggleFold` would re-target an unfolded group's collapse header
            (TimelineItemAction::ToggleFold, ActionTarget::Entry) => {
                self.reveal_timeline_entry(idx);
                self.scrollback.toggle_fold_selected();
                InputOutcome::Changed
            }
            // Group-wide copy and viewer need a member-concatenation rule first (deferred package S-K)
            (
                TimelineItemAction::CopyContent
                | TimelineItemAction::CopyMeta
                | TimelineItemAction::OpenViewer,
                ActionTarget::Group,
            ) => InputOutcome::Unchanged,
            (TimelineItemAction::CopyContent, ActionTarget::Entry) => {
                self.reveal_timeline_entry(idx);
                InputOutcome::Action(Action::CopyBlockContent)
            }
            (TimelineItemAction::CopyMeta, ActionTarget::Entry) => {
                self.reveal_timeline_entry(idx);
                InputOutcome::Action(Action::CopyBlockMeta)
            }
            (TimelineItemAction::OpenViewer, ActionTarget::Entry) => {
                self.reveal_timeline_entry(idx);
                InputOutcome::Action(Action::OpenBlockViewer)
            }
            (TimelineItemAction::Rewind, ActionTarget::Entry | ActionTarget::Group) => {
                self.scrollback.set_selected(Some(idx));
                InputOutcome::Action(Action::Rewind)
            }
        }
    }
}

/// Whether an action addresses one entry or a whole fold group (then keyed by its first member).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionTarget {
    Entry,
    Group,
}

impl AgentView {
    /// Alt+click on a chat entry: open the panel on it with the panel focused.
    pub(crate) fn open_timeline_panel_on_click(&mut self, row: u16) -> Option<InputOutcome> {
        let idx = self
            .scrollback
            .entry_index_at_screen_row(row, self.pane_areas.scrollback)?;
        self.scrollback.set_selected(Some(idx));
        Some(self.open_timeline_panel_at(Some(idx)))
    }
}

#[cfg(test)]
#[path = "timeline_panel_tests.rs"]
mod tests;
