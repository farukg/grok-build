use super::AgentView;
use super::removal_verdict::{RemovalVerdict, confirm_label, counted, items_phrase};
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::scrollback::state::{TimelineRow, TimelineRowKind};
use crossterm::event::{KeyCode, KeyEvent};
use xai_grok_shell::session::ContextItemRef;

impl AgentView {
    pub(super) fn run_timeline_item_remove(&mut self, row: TimelineRow, key: &KeyEvent) -> InputOutcome {
        let removal_in_flight = self
            .session
            .session_id
            .as_ref()
            .is_some_and(|session_id| self.pending_removed_context_items.contains(session_id));
        if removal_in_flight {
            self.show_toast("A removal is already in progress");
            return InputOutcome::Changed;
        }
        match self.verdict_for_row(row) {
            RemovalVerdict::NotRemovable(reason) => {
                self.show_toast(&reason.text());
                InputOutcome::Changed
            }
            RemovalVerdict::Removable { refs, skipped } => {
                self.show_toast(&confirm_hint(row, &refs, skipped));
                let label = confirm_label(&refs);
                InputOutcome::ArmPending {
                    action: Action::RemoveContextItems(refs),
                    shortcut: crate::input::key::KeyShortcut::from(*key),
                    label: Some(label),
                    ttl: crate::app::app_view::esc_double_press_ttl(),
                }
            }
        }
    }
}

/// Toast shown when a removal is armed: names what the second press does.
fn confirm_hint(row: TimelineRow, refs: &[ContextItemRef], skipped: usize) -> String {
    let what = match (row.kind, refs) {
        (TimelineRowKind::Turn { turn_idx, .. }, [ContextItemRef::Turn { .. }]) => format!("turn {}", turn_idx + 1),
        (_, [ContextItemRef::ToolExchange { .. }]) => "1 tool call".to_owned(),
        (_, refs) => items_phrase(refs.len()),
    };
    match skipped {
        0 => format!("Press again to remove {what} from the model context"),
        skipped => format!(
            "Press again to remove {what} from the model context ({} not removable, left out)",
            counted(skipped, "entry", "entries")
        ),
    }
}

pub(crate) fn is_remove_key(key: &KeyEvent) -> bool {
    key.modifiers.is_empty() && matches!(key.code, KeyCode::Char('d') | KeyCode::Delete)
}
