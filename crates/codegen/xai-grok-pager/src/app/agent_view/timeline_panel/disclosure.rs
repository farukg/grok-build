//! Opening and closing outline branches: tree steps, Space, bulk gestures and clicks.
//!
//! Group rows are applied through the scrollback (the chat shows the same fold); turn rows live in the panel's own disclosure.

use super::AgentView;
use crate::app::app_view::InputOutcome;
use crate::scrollback::entry::EntryId;
use crate::scrollback::state::{TimelineRow, TimelineRowKey, TimelineRowKind};
use crate::views::timeline_panel::gesture::{BulkRequest, RowClick};
use crate::views::tree_nav::{BulkDirection, TreeStep};

impl AgentView {
    /// Carry out a tree step of the panel: move the cursor (the chat follows) or open/close a branch.
    pub(super) fn apply_timeline_step(&mut self, step: TreeStep) -> InputOutcome {
        match step {
            TreeStep::Stay => {}
            TreeStep::MoveTo(index) => {
                let moved = self
                    .timeline_mode
                    .panel_mut()
                    .is_some_and(|panel| panel.set_cursor(index));
                if moved {
                    self.sync_timeline_cursor_to_chat();
                }
            }
            TreeStep::Expand(index) => {
                self.set_timeline_branches(&[index], true);
            }
            TreeStep::Collapse(index) => {
                self.set_timeline_branches(&[index], false);
            }
        }
        InputOutcome::Changed
    }

    /// Space, and a click on the disclosure glyph: open or close the branch under the cursor, or say why it cannot be.
    pub(super) fn toggle_timeline_cursor_branch(&mut self) -> InputOutcome {
        let Some(panel) = self.timeline_mode.panel() else {
            return InputOutcome::Changed;
        };
        match panel.toggle_step() {
            TreeStep::Stay => {
                let reason = panel.leaf_reason();
                self.show_toast(reason);
                InputOutcome::Changed
            }
            step @ (TreeStep::MoveTo(_) | TreeStep::Expand(_) | TreeStep::Collapse(_)) => {
                self.apply_timeline_step(step)
            }
        }
    }

    /// Shift/Ctrl+Enter, `*`, `+`, `-` and their clicks: open or close every branch the request reaches.
    pub(super) fn apply_timeline_bulk(&mut self, request: BulkRequest) -> InputOutcome {
        let Some(panel) = self.timeline_mode.panel() else {
            return InputOutcome::Changed;
        };
        let change = panel.plan_bulk(request);
        if change.indices.is_empty() {
            self.show_toast(request.goal.nothing_to_do());
            return InputOutcome::Changed;
        }
        let expand = matches!(change.direction, BulkDirection::Expand);
        self.set_timeline_branches(&change.indices, expand);
        InputOutcome::Changed
    }

    /// A left click on a row whose cursor and focus are already set.
    pub(super) fn apply_timeline_click(&mut self, click: RowClick) -> InputOutcome {
        match click {
            RowClick::Select => InputOutcome::Changed,
            RowClick::Toggle => self.toggle_timeline_cursor_branch(),
            RowClick::Bulk(request) => self.apply_timeline_bulk(request),
        }
    }

    /// Set the branches at the outline rows `indices` open or closed; true when anything changed.
    /// A cursor left on a hidden row moves to its visible ancestor and the chat follows.
    pub(super) fn set_timeline_branches(&mut self, indices: &[usize], expand: bool) -> bool {
        let Some(panel) = self.timeline_mode.panel_mut() else {
            return false;
        };
        let cursor_before = panel.cursor_row().map(TimelineRow::key);
        let cursor_entry = panel.cursor_row().map(TimelineRow::entry_id);
        let mut changed = false;
        let mut changed_turns: Vec<EntryId> = Vec::new();
        let mut groups: Vec<(EntryId, bool)> = Vec::new();
        for &index in indices {
            let Some(row) = panel.row(index).copied() else {
                continue;
            };
            match row.kind {
                TimelineRowKind::Turn { prompt_id, .. } => {
                    if panel.set_turn_expanded(prompt_id, expand) {
                        changed = true;
                        changed_turns.push(prompt_id);
                    }
                }
                TimelineRowKind::Group { first_id } => {
                    if self.scrollback.index_of_id(first_id).is_some() {
                        groups.push((first_id, expand));
                    }
                }
                TimelineRowKind::Entry { .. } => {}
            }
        }
        let anchor = cursor_entry
            .or_else(|| groups.first().map(|&(first_id, _)| first_id))
            .and_then(|id| self.scrollback.index_of_id(id));
        if let Some(anchor) = anchor.filter(|_| !groups.is_empty()) {
            changed |= self.scrollback.set_group_expansion(&groups, anchor);
        }
        let Some(panel) = self.timeline_mode.panel_mut() else {
            return changed;
        };
        panel.refresh(&self.scrollback);
        let cursor_after: Option<TimelineRowKey> = panel.cursor_row().map(TimelineRow::key);
        if cursor_after != cursor_before {
            self.sync_timeline_cursor_to_chat();
        }
        self.save_turn_states(&changed_turns);
        changed
    }
}
