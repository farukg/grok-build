//! Cursor, paging and disclosure operations of the outline panel over its visible rows.

use super::TimelinePanelState;
use super::disclosure::TurnDisclosure;
use super::gesture::{BulkGoal, BulkRequest, PanelNav};
use super::outline_view::OutlineView;
use super::row_line::INDENT_WIDTH;
use crate::app::panel_settings::DisclosureState;
use crate::scrollback::entry::EntryId;
use crate::scrollback::state::{ScrollbackState, TimelineRow, TimelineRowKey, TimelineRowKind};
use crate::views::tree_nav::{
    BulkChange, TreeStep, bulk_set, bulk_toggle, navigate, toggle, visible_anchor,
};

/// A chat-driven follow that landed on a hidden row and was parked on its visible ancestor.
/// While nothing changed, the same follow target needs no new search of the outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ParkedFollow {
    generation: u64,
    turns_version: u64,
    key: TimelineRowKey,
    cursor: usize,
}

impl TimelinePanelState {
    pub fn row(&self, index: usize) -> Option<&TimelineRow> {
        self.rows.get(index)
    }

    /// Visible rows a page key moves: one less than the panel height, so one row stays in view.
    pub fn page_rows(&self) -> usize {
        usize::from(self.area.height.max(2) - 1)
    }

    /// The step `nav` takes from the cursor.
    pub fn navigate(&self, nav: PanelNav) -> TreeStep {
        if self.cursor_row().is_none() {
            let row = match nav {
                PanelNav::Up | PanelNav::End | PanelNav::PageUp => self.view.visible().last(),
                PanelNav::Down | PanelNav::Home | PanelNav::PageDown => self.view.visible().first(),
                PanelNav::Left | PanelNav::Right => None,
            };
            return row.copied().map(TreeStep::MoveTo).unwrap_or(TreeStep::Stay);
        }
        navigate(
            self.view.tree(),
            self.cursor,
            nav.tree_key(self.page_rows()),
        )
    }

    pub(crate) fn end_nav_is_stay(&self) -> bool {
        matches!(self.navigate(PanelNav::End), TreeStep::Stay)
    }

    /// Open or close the branch under the cursor; `Stay` on a leaf.
    pub fn toggle_step(&self) -> TreeStep {
        toggle(self.view.tree(), self.cursor)
    }

    /// Why the cursor row cannot be opened or closed.
    pub fn leaf_reason(&self) -> &'static str {
        match self.cursor_row().map(|row| row.kind) {
            Some(TimelineRowKind::Turn { .. }) => "This turn has no entries to fold",
            Some(TimelineRowKind::Entry { .. }) => {
                "An entry has no children to fold (e folds its content)"
            }
            Some(TimelineRowKind::Group { .. }) | None => "Nothing to fold here",
        }
    }

    /// The branches a bulk gesture changes, and how, relative to the cursor row.
    pub fn plan_bulk(&self, request: BulkRequest) -> BulkChange {
        match request.goal {
            BulkGoal::Toggle => {
                bulk_toggle(self.view.tree(), self.cursor, request.scope, request.anchor)
            }
            BulkGoal::Set(direction) => bulk_set(
                self.view.tree(),
                self.cursor,
                request.scope,
                direction,
                request.anchor,
            ),
        }
    }

    /// Whether screen column `col` is on the disclosure glyph (or the gap after it) of branch row `index`.
    pub fn is_on_glyph(&self, col: u16, index: usize) -> bool {
        let is_branch = self
            .view
            .tree()
            .get(index)
            .is_some_and(|row| row.branch.is_some());
        let Some(row) = self.rows.get(index) else {
            return false;
        };
        let glyph_x = self
            .area
            .x
            .saturating_add(u16::from(row.depth).saturating_mul(INDENT_WIDTH));
        is_branch && (glyph_x..glyph_x.saturating_add(2)).contains(&col)
    }

    pub fn turn_disclosure(&self) -> &TurnDisclosure {
        &self.turns
    }

    /// Replace the folded turns, e.g. with a restored value; the outline follows on the next [`Self::refresh`].
    pub fn apply_turn_disclosure(&mut self, turns: TurnDisclosure) {
        if self.turns != turns {
            self.turns = turns;
            self.turns_version = self.turns_version.wrapping_add(1);
        }
    }

    /// Whether the turn's state changed.
    pub fn set_turn_expanded(&mut self, prompt: EntryId, expanded: bool) -> bool {
        let changed = self.turns.set_expanded(prompt, expanded);
        if changed {
            self.turns_version = self.turns_version.wrapping_add(1);
        }
        changed
    }

    /// Makes `state` the state of every turn without one of its own; the turns whose own state equals it are returned.
    pub fn set_turn_default(&mut self, state: DisclosureState) -> Vec<EntryId> {
        let before = self.turns.default_state();
        let following = self.turns.set_default(state);
        if before != state {
            self.turns_version = self.turns_version.wrapping_add(1);
        }
        following
    }

    /// Rebuild the derived tree view when the outline or the turn disclosure moved, then keep the cursor on a visible row.
    pub(super) fn refresh_view(&mut self, scrollback: &ScrollbackState) {
        if !self.view.is_current(self.generation, self.turns_version) {
            self.view = OutlineView::build(
                &self.rows,
                scrollback,
                &self.turns,
                self.generation,
                self.turns_version,
            );
            #[cfg(test)]
            {
                self.view_builds += 1;
            }
        }
        self.repair_cursor();
    }

    /// Move a cursor hidden by a collapse onto its outermost visible ancestor; true when it moved.
    /// The visible case costs one binary search, so this can run every frame.
    pub(super) fn repair_cursor(&mut self) -> bool {
        if self.view.position_of(self.cursor).is_some() {
            return false;
        }
        match visible_anchor(self.view.tree(), self.cursor) {
            Some(anchor) if anchor != self.cursor => {
                self.cursor = anchor;
                true
            }
            Some(_) | None => false,
        }
    }

    /// Whether following `key` already parked the cursor on its visible ancestor.
    pub(super) fn is_parked_on(&self, key: TimelineRowKey) -> bool {
        self.parked == Some(self.parked_at(key))
    }

    /// After a follow: repair a cursor that landed on a hidden row and remember that `key` is parked.
    pub(super) fn park_if_hidden(&mut self, key: TimelineRowKey) {
        self.parked = self.repair_cursor().then(|| self.parked_at(key));
    }

    fn parked_at(&self, key: TimelineRowKey) -> ParkedFollow {
        ParkedFollow {
            generation: self.generation,
            turns_version: self.turns_version,
            key,
            cursor: self.cursor,
        }
    }

    /// Number of times the tree view was built.
    #[cfg(test)]
    pub fn view_builds(&self) -> usize {
        self.view_builds
    }
}
