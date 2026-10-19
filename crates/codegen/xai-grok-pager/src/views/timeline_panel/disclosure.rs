//! Which turns of the outline are folded.
//!
//! Turn rows have no scrollback state, so the panel owns their disclosure: one state every turn follows, plus the turns
//! that were given a state of their own. Saving it belongs to the app layer, which keys turns by prompt ordinal.

use std::collections::HashMap;

use crate::app::panel_settings::DisclosureState;
use crate::scrollback::entry::EntryId;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnDisclosure {
    /// The state of every turn without one of its own.
    follow: DisclosureState,
    /// Turns (by prompt entry) with a state of their own; never equal to `follow`.
    own: HashMap<EntryId, DisclosureState>,
}

impl TurnDisclosure {
    /// Every turn in `state`.
    pub fn following(state: DisclosureState) -> Self {
        Self {
            follow: state,
            own: HashMap::new(),
        }
    }

    /// The state turns have unless they were given one of their own.
    pub fn default_state(&self) -> DisclosureState {
        self.follow
    }

    pub fn is_expanded(&self, prompt: EntryId) -> bool {
        self.own
            .get(&prompt)
            .copied()
            .unwrap_or(self.follow)
            .is_expanded()
    }

    /// Whether the turn's state changed.
    pub fn set_expanded(&mut self, prompt: EntryId, expanded: bool) -> bool {
        let state = DisclosureState::from_expanded(expanded);
        if state == self.follow {
            self.own.remove(&prompt).is_some()
        } else {
            self.own.insert(prompt, state) != Some(state)
        }
    }

    /// Makes `state` the default. Turns whose own state equals it follow the default from now on; they are returned.
    pub fn set_default(&mut self, state: DisclosureState) -> Vec<EntryId> {
        self.follow = state;
        let following: Vec<EntryId> = self
            .own
            .iter()
            .filter(|&(_, own)| *own == state)
            .map(|(&prompt, _)| prompt)
            .collect();
        self.own.retain(|_, own| *own != state);
        following
    }
}
