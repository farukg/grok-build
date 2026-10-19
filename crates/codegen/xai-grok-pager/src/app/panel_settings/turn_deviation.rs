//! Saving turn disclosure as the smallest session edit.

use super::error::{SaveError, SaveOutcome};
use super::store::PanelSettingsStore;
use super::types::{DisclosureState, Edit, Layer, PromptOrdinal};

impl PanelSettingsStore {
    /// Makes the session layer yield `wanted` for each turn, in one write. A state equal to the turn default in force
    /// (session, else global, else built-in) removes the turn's exception; any other state is stored as one.
    /// Needs an attached session.
    pub fn set_turn_deviations(
        &mut self,
        wanted: &[(PromptOrdinal, DisclosureState)],
    ) -> Result<SaveOutcome, SaveError> {
        let default = self.effective().outline_turns();
        let edits: Vec<Edit> = wanted
            .iter()
            .map(|&(prompt, state)| Edit::Turn(prompt, (state != default).then_some(state)))
            .collect();
        self.save(Layer::Session, &edits)
    }

    /// Drops the session's turn exceptions for `from` and every later prompt; a rewind hands those numbers to new
    /// prompts. Without an attached session there is nothing to drop.
    pub fn clear_turns_from(&mut self, from: PromptOrdinal) -> Result<SaveOutcome, SaveError> {
        let edits: Vec<Edit> = self
            .session()
            .map(|session| session.turns_from(from))
            .unwrap_or_default()
            .into_iter()
            .map(|prompt| Edit::Turn(prompt, None))
            .collect();
        if edits.is_empty() {
            return Ok(SaveOutcome::Unchanged);
        }
        self.save(Layer::Session, &edits)
    }
}
