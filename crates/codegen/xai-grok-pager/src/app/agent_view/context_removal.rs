use super::AgentView;
use crate::app::actions::{Action, Effect, InputOutcome};
use crate::scrollback::state::{TimelineRow, TimelineRowKind};
use crossterm::event::{KeyCode, KeyEvent};

impl AgentView {
    pub(super) fn run_timeline_item_remove(&mut self, row: TimelineRow, key: &KeyEvent) -> InputOutcome {
        let refs = self.timeline_remove_refs(row);
        if refs.is_empty() {
            self.show_toast("This entry cannot be removed from model context");
            return InputOutcome::Changed;
        }
        InputOutcome::ArmPending {
            action: Action::RemoveContextItems(refs),
            shortcut: crate::input::key::KeyShortcut::from(*key),
            label: Some("remove from context"),
            ttl: crate::app::app_view::esc_double_press_ttl(),
        }
    }

    pub(super) fn timeline_remove_refs(&self, row: TimelineRow) -> Vec<xai_chat_state::ContextItemRef> {
        match row.kind {
            TimelineRowKind::Turn { .. } | TimelineRowKind::Entry { .. } => {
                let Some(idx) = self.scrollback.index_of_id(row.entry_id()) else { return Vec::new() };
                match self.scrollback.get(idx).map(|entry| &entry.block) {
                    Some(crate::scrollback::block::RenderBlock::UserPrompt(prompt)) => prompt.prompt_index
                        .map(|prompt_index| vec![xai_chat_state::ContextItemRef::Turn { prompt_index }])
                        .unwrap_or_default(),
                    Some(crate::scrollback::block::RenderBlock::ToolCall(_)) => self.session.tracker
                        .context_tool_call_id_for_entry(row.entry_id())
                        .map(|tool_call_id| vec![xai_chat_state::ContextItemRef::ToolExchange { tool_call_id }])
                        .unwrap_or_default(),
                    _ => Vec::new(),
                }
            }
            TimelineRowKind::Group { first_id } => {
                let Some(first) = self.scrollback.index_of_id(first_id) else { return Vec::new() };
                let Some(span) = self.scrollback.span_at(first) else { return Vec::new() };
                span.range.filter_map(|idx| match self.scrollback.get(idx).map(|entry| &entry.block) {
                    Some(crate::scrollback::block::RenderBlock::UserPrompt(prompt)) => prompt.prompt_index
                        .map(|prompt_index| xai_chat_state::ContextItemRef::Turn { prompt_index }),
                    Some(crate::scrollback::block::RenderBlock::ToolCall(_)) => self.scrollback.get(idx)
                        .and_then(|entry| self.session.tracker.context_tool_call_id_for_entry(entry.id))
                        .map(|tool_call_id| xai_chat_state::ContextItemRef::ToolExchange { tool_call_id }),
                    _ => None,
                }).collect()
            }
        }
    }

    pub(super) fn dispatch_remove_context_items(&mut self, refs: Vec<xai_chat_state::ContextItemRef>) {
        let Some(session_id) = self.session.session_id.clone() else {
            self.show_toast("Session is not connected");
            return;
        };
        self.pending_removed_context_items.push((session_id.clone(), refs.clone()));
        self.pending_effects.push(Effect::RemoveContextItems { session_id, items: refs });
    }
}

pub(crate) fn is_remove_key(key: &KeyEvent) -> bool {
    key.modifiers.is_empty() && matches!(key.code, KeyCode::Char('d') | KeyCode::Delete)
}
