//! Giving a kind back to its blocks' own display policy.

use super::*;

impl ScrollbackState {
    /// Drops the user's form for `kind`: blocks that appear from now on follow their own display policy again.
    /// Entries already on screen keep the mode they have; a replayed session rebuilds them under the new policy.
    pub fn clear_kind_default(&mut self, kind: MessageKind) {
        self.display_defaults.remove(kind);
    }
}
