//! What the outline footer offers for the cursor row, without per-frame work that grows with the outline.

use super::AgentView;
use crate::app::agent_view::removal_verdict::{Members, RemovalVerdict, RowFacts, removal_verdict};
use crate::scrollback::entry::EntryId;
use crate::scrollback::state::{TimelineRow, TimelineRowKind};
use crate::views::timeline_panel::footer_memo::GroupRemovableKey;
use crate::views::timeline_panel::hint::FooterContext;

impl AgentView {
    /// The cursor row and whether `d` can remove it.
    pub(in crate::app::agent_view) fn timeline_footer_context(&mut self) -> FooterContext {
        let Some(row) = self
            .timeline_mode
            .panel()
            .and_then(|panel| panel.cursor_row())
            .copied()
        else {
            return FooterContext {
                cursor: None,
                removable: false,
            };
        };
        let removable = match row.kind {
            TimelineRowKind::Turn { .. } | TimelineRowKind::Entry { .. } => {
                matches!(self.verdict_for_row(row), RemovalVerdict::Removable { .. })
            }
            TimelineRowKind::Group { first_id } => self.group_has_removable_member(row, first_id),
        };
        FooterContext {
            cursor: Some(row.kind),
            removable,
        }
    }

    /// A group row is removable when any member is; the answer is kept until something it depends on moves.
    fn group_has_removable_member(&mut self, row: TimelineRow, first_id: EntryId) -> bool {
        let key = GroupRemovableKey {
            group: row.key(),
            outline_generation: self.scrollback.outline_generation(),
            content_generation: self.scrollback.content_generation(),
            has_session: self.session.session_id.is_some(),
        };
        let remembered = self
            .timeline_mode
            .panel()
            .and_then(|panel| panel.remembered_group_removable(key));
        if let Some(removable) = remembered {
            return removable;
        }
        let removable = self.any_removable_member(first_id, key.has_session);
        if let Some(panel) = self.timeline_mode.panel_mut() {
            panel.remember_group_removable(key, removable);
        }
        removable
    }

    /// Stops at the first removable member, so a group costs more than one entry only while its head is refused.
    fn any_removable_member(&self, first_id: EntryId, has_session: bool) -> bool {
        let Some(span) = self
            .scrollback
            .index_of_id(first_id)
            .and_then(|first| self.scrollback.span_at(first))
        else {
            return false;
        };
        span.range
            .clone()
            .filter_map(|idx| self.scrollback.get(idx))
            .any(|entry| {
                let facts = RowFacts {
                    has_session,
                    members: Members::One(self.removal_entry_facts(entry)),
                };
                matches!(removal_verdict(&facts), RemovalVerdict::Removable { .. })
            })
    }
}
