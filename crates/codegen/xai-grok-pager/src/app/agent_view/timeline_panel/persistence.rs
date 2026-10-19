//! Saving the F6 turn folds and restoring them when the panel opens.
//!
//! A turn is keyed by its shell prompt number (`UserPromptBlock::prompt_index`): the shell keeps it across restarts,
//! replay and the removal of other turns; a rewind hands its numbers to new prompts, so it drops the folds saved under
//! them. A turn without one is not saved: its fold lives in the panel only and is reset when the session binds or
//! changes (the restore applies the stored folds wholesale). The fold of a single turn belongs to the
//! session, so it goes to the session layer whatever the shared edit layer says; the turn default ("turns start open or
//! folded", key `t`) follows the edit layer. Group folds are live scrollback state shared with the chat and are not saved.

use super::AgentView;
use crate::app::agent_view::panel_persistence::OVERRULED;
use crate::app::app_view::InputOutcome;
use crate::app::panel_settings::{DisclosureState, PromptOrdinal, Setting};
use crate::scrollback::block::RenderBlock;
use crate::scrollback::entry::{EntryId, ScrollbackEntry};
use crate::scrollback::state::TimelineRow;
use crate::views::timeline_panel::disclosure::TurnDisclosure;

fn prompt_ordinal(entry: &ScrollbackEntry) -> Option<PromptOrdinal> {
    let RenderBlock::UserPrompt(prompt) = &entry.block else {
        return None;
    };
    prompt
        .prompt_index
        .filter(|_| !prompt.is_interjection && !prompt.is_bash)
        .and_then(PromptOrdinal::from_prompt_index)
}

const fn turns_start(state: DisclosureState) -> &'static str {
    match state {
        DisclosureState::Expanded => "Turns start open",
        DisclosureState::Collapsed => "Turns start folded",
    }
}

impl AgentView {
    /// The turn folds a panel opens with: the stored turn default and this session's stored exceptions.
    pub(super) fn restored_turn_disclosure(&self) -> TurnDisclosure {
        let stored = self.panel_persistence.effective();
        let mut turns = TurnDisclosure::following(stored.outline_turns());
        for (id, entry) in self.scrollback.iter_entries() {
            if let Some(ordinal) = prompt_ordinal(entry) {
                turns.set_expanded(id, stored.turn(ordinal).is_expanded());
            }
        }
        turns
    }

    /// Puts an open panel on the stored folds, e.g. after the view moved to another session.
    pub(in crate::app::agent_view) fn restore_f6_panel_settings(&mut self) {
        let restored = self.restored_turn_disclosure();
        if let Some(panel) = self.timeline_mode.panel_mut() {
            panel.apply_turn_disclosure(restored);
            panel.refresh(&self.scrollback);
        }
    }

    /// A rewind to `target` hands the prompt numbers from `target` upwards to new prompts; folds saved under them
    /// belonged to the discarded turns.
    pub(crate) fn clear_turn_folds_from(&mut self, target: usize) {
        let Some(from) = PromptOrdinal::from_prompt_index(target) else {
            return;
        };
        let Some(store) = self.panel_persistence.store_mut() else {
            return;
        };
        let result = store.clear_turns_from(from);
        self.report_panel_save(result);
    }

    /// Saves the current fold of each turn in `turns`; only a fold that differs from the turn default in force is kept.
    pub(super) fn save_turn_states(&mut self, turns: &[EntryId]) {
        let Some(panel) = self.timeline_mode.panel() else {
            return;
        };
        let wanted: Vec<_> = turns
            .iter()
            .filter_map(|&id| {
                let ordinal = prompt_ordinal(self.scrollback.get_by_id(id)?)?;
                let state = DisclosureState::from_expanded(panel.turn_disclosure().is_expanded(id));
                Some((ordinal, state))
            })
            .collect();
        if wanted.is_empty() {
            return;
        }
        // Without a session nothing is numbered yet, and turn folds have no layer to live in.
        let Some(store) = self
            .panel_persistence
            .store_mut()
            .filter(|store| store.session_key().is_some())
        else {
            return;
        };
        let result = store.set_turn_deviations(&wanted);
        self.report_panel_save(result);
    }

    /// `t`: flip whether turns start open or folded, for every turn without a fold of its own.
    pub(super) fn toggle_turn_default(&mut self) -> InputOutcome {
        let Some(panel) = self.timeline_mode.panel_mut() else {
            return InputOutcome::Changed;
        };
        let cursor_before = panel.cursor_row().map(TimelineRow::key);
        let wanted = panel.turn_disclosure().default_state().toggled();
        let following = panel.set_turn_default(wanted);
        self.show_toast(turns_start(wanted));
        if self.save_panel_settings(&[Setting::OutlineTurns(wanted)]) {
            self.settle_turn_default(wanted, &following);
        }
        let Some(panel) = self.timeline_mode.panel_mut() else {
            return InputOutcome::Changed;
        };
        panel.refresh(&self.scrollback);
        let cursor_after = panel.cursor_row().map(TimelineRow::key);
        if cursor_after != cursor_before {
            self.sync_timeline_cursor_to_chat();
        }
        InputOutcome::Changed
    }

    /// After a stored default: drop the turn folds that now equal it, or, when this session's own default
    /// overrules the saved one, show what a restart would show and say so.
    fn settle_turn_default(&mut self, wanted: DisclosureState, following: &[EntryId]) {
        if self.panel_persistence.effective().outline_turns() == wanted {
            self.save_turn_states(following);
            return;
        }
        let in_force = self.restored_turn_disclosure();
        if let Some(panel) = self.timeline_mode.panel_mut() {
            panel.apply_turn_disclosure(in_force);
        }
        self.show_toast(OVERRULED);
    }
}
