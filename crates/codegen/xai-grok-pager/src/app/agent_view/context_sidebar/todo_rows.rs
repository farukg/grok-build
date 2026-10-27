//! The F7 projection of the Todos pane: wrapped, budget-bounded rows from the shared todo owner.

use crate::views::sidebar::{SidebarRow, SidebarRowSource};
use crate::views::todo_pane::{SidebarProjection, TodoPane};

/// A [`TodoPane`] as the sidebar's Todos section: one projection at one text width serves the
/// row count and every painted row of a draw.
pub(super) struct SidebarTodos<'a> {
    pane: &'a TodoPane,
    projection: SidebarProjection,
}

impl<'a> SidebarTodos<'a> {
    pub(super) fn new(pane: &'a TodoPane, text_width: u16) -> Self {
        Self {
            pane,
            projection: pane.sidebar_projection(text_width),
        }
    }

    pub(super) fn count(&self) -> usize {
        self.projection.row_count()
    }
}

impl SidebarRowSource for SidebarTodos<'_> {
    fn row(&self, index: usize) -> Option<SidebarRow<'static>> {
        self.pane.sidebar_row_of(&self.projection, index)
    }
}

/// Text width of a sidebar row: the column without its border and the scrollbar lane,
/// matching the row renderer's `body.width - 1`.
pub(super) fn text_width(column: u16) -> u16 {
    column.saturating_sub(3)
}
