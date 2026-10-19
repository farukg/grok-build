//! Shell turns the scrollback cannot show. Auto-wake and goal prompts carry a shell prompt number but have no prompt block,
//! so only the position of the next hidden turn tells where the rows of the preceding visible turn end.

use crate::acp::meta::user_message_chunk_meta;
use crate::scrollback::entry::EntryId;
use crate::scrollback::state::ScrollbackState;
use agent_client_protocol as acp;
use std::collections::HashSet;

#[derive(Debug, Default)]
pub(crate) struct TurnLedger {
    /// Entries directly followed by a numbered shell turn that has no prompt block.
    hidden_turn_starts_after: HashSet<EntryId>,
}

impl TurnLedger {
    /// A user chunk the scrollback hides: when the shell numbered it, a turn starts after the newest entry.
    pub(crate) fn note_hidden_user_chunk(
        &mut self,
        chunk: &acp::ContentChunk,
        scrollback: &ScrollbackState,
    ) {
        let numbered = chunk
            .meta
            .as_ref()
            .is_some_and(|meta| meta.contains_key(user_message_chunk_meta::PROMPT_INDEX));
        if let (true, Some(last)) = (numbered, scrollback.last()) {
            self.hidden_turn_starts_after.insert(last.id);
        }
    }

    /// Whether the visible turn containing `id` ends with that entry because a hidden turn follows it.
    pub(crate) fn turn_ends_after(&self, id: EntryId) -> bool {
        self.hidden_turn_starts_after.contains(&id)
    }

    /// Hand the boundaries of entries about to leave the scrollback to the nearest earlier entry that stays.
    /// Call before the entries are removed.
    pub(crate) fn carry_to_survivors(
        &mut self,
        scrollback: &ScrollbackState,
        removed: &HashSet<EntryId>,
    ) {
        for id in removed {
            if !self.hidden_turn_starts_after.remove(id) {
                continue;
            }
            let survivor = scrollback.index_of_id(*id).and_then(|index| {
                (0..index).rev().find_map(|earlier| {
                    scrollback
                        .get(earlier)
                        .map(|entry| entry.id)
                        .filter(|candidate| !removed.contains(candidate))
                })
            });
            if let Some(survivor) = survivor {
                self.hidden_turn_starts_after.insert(survivor);
            }
        }
    }
}

#[cfg(test)]
#[path = "turn_ledger_tests.rs"]
mod tests;
