//! Footer inputs that change only on a structural event, so a frame does not recompute them.

use super::TimelinePanelState;
use crate::scrollback::state::TimelineRowKey;

/// What the removal hint of a group row depends on. Finding out walks the group's members, so the answer is reused
/// until the outline, an entry's content, or the session binding moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupRemovableKey {
    pub group: TimelineRowKey,
    pub outline_generation: u64,
    pub content_generation: u64,
    pub has_session: bool,
}

/// What the footer height depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FooterRowsKey {
    pub column_width: u16,
    pub column_height: u16,
    pub status: &'static str,
}

#[derive(Debug, Default)]
pub(super) struct FooterMemo {
    group_removable: Option<(GroupRemovableKey, bool)>,
    rows: Option<(FooterRowsKey, u16)>,
}

impl TimelinePanelState {
    /// Whether a member of the group can be removed, if that was worked out for exactly `key`.
    pub fn remembered_group_removable(&self, key: GroupRemovableKey) -> Option<bool> {
        self.footer
            .group_removable
            .filter(|(held, _)| *held == key)
            .map(|(_, removable)| removable)
    }

    pub fn remember_group_removable(&mut self, key: GroupRemovableKey, removable: bool) {
        self.footer.group_removable = Some((key, removable));
    }

    /// The footer height for `key`; `compute` runs only when `key` changed.
    pub fn footer_rows_for(&mut self, key: FooterRowsKey, compute: impl FnOnce() -> u16) -> u16 {
        let remembered = self
            .footer
            .rows
            .filter(|(held, _)| *held == key)
            .map(|(_, rows)| rows);
        match remembered {
            Some(rows) => rows,
            None => {
                let rows = compute();
                self.footer.rows = Some((key, rows));
                rows
            }
        }
    }
}
