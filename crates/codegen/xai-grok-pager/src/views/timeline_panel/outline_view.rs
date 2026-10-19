//! Tree shape and visible rows of the outline, derived once per outline generation and turn-disclosure version.
//!
//! The scrollback span is the single source of truth for group rows (so the chat and the panel agree); turn rows read the panel's own disclosure.

use super::disclosure::TurnDisclosure;
use crate::scrollback::entry::EntryId;
use crate::scrollback::state::{ScrollbackState, TimelineRow, TimelineRowKind};
use crate::views::tree_nav::{TreeRow, visible_rows};

#[derive(Debug, Default)]
pub(super) struct OutlineView {
    /// `(outline generation, turn-disclosure version)` this view was built for; `None` before the first build.
    built_for: Option<(u64, u64)>,
    tree: Vec<TreeRow>,
    /// Indices of the visible rows into the outline rows, ascending.
    visible: Vec<usize>,
}

impl OutlineView {
    pub(super) fn build(
        rows: &[TimelineRow],
        scrollback: &ScrollbackState,
        turns: &TurnDisclosure,
        generation: u64,
        turns_version: u64,
    ) -> Self {
        let tree: Vec<TreeRow> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let has_children = rows
                    .get(index + 1)
                    .is_some_and(|next| next.depth > row.depth);
                TreeRow {
                    depth: row.depth,
                    branch: branch_state(row, has_children, scrollback, turns),
                }
            })
            .collect();
        let visible = visible_rows(&tree);
        Self {
            built_for: Some((generation, turns_version)),
            tree,
            visible,
        }
    }

    pub(super) fn is_current(&self, generation: u64, turns_version: u64) -> bool {
        self.built_for == Some((generation, turns_version))
    }

    pub(super) fn tree(&self) -> &[TreeRow] {
        &self.tree
    }

    pub(super) fn visible(&self) -> &[usize] {
        &self.visible
    }

    /// Position of outline row `row` among the visible rows.
    pub(super) fn position_of(&self, row: usize) -> Option<usize> {
        self.visible.binary_search(&row).ok()
    }

    /// Outline row shown at visible `position`.
    pub(super) fn row_at(&self, position: usize) -> Option<usize> {
        self.visible.get(position).copied()
    }
}

/// `Some(expanded)` for a row with children, `None` for a leaf.
fn branch_state(
    row: &TimelineRow,
    has_children: bool,
    scrollback: &ScrollbackState,
    turns: &TurnDisclosure,
) -> Option<bool> {
    match row.kind {
        TimelineRowKind::Turn { prompt_id, .. } => {
            has_children.then(|| turns.is_expanded(prompt_id))
        }
        TimelineRowKind::Group { first_id } => Some(group_expanded(scrollback, first_id)),
        TimelineRowKind::Entry { .. } => None,
    }
}

/// A group without a live span (stale outline) counts as open, so its members stay reachable.
fn group_expanded(scrollback: &ScrollbackState, first_id: EntryId) -> bool {
    scrollback
        .index_of_id(first_id)
        .and_then(|idx| scrollback.span_at(idx))
        .is_none_or(|span| span.expanded)
}
