//! Applying the effective F7 display settings through one lifecycle resolver.

use super::*;

pub(super) fn resolve_mode(
    block: &RenderBlock,
    form: Option<DisplayForm>,
    legacy_edit_expanded: bool,
) -> DisplayMode {
    match (block, form) {
        (_, Some(form)) => DisplayMode::from(form),
        (RenderBlock::ToolCall(ToolCallBlock::Edit(edit)), None) => {
            edit_default_display_mode(legacy_edit_expanded, edit)
        }
        (block, None) => block.default_display_mode(),
    }
}

impl ScrollbackState {
    pub fn clear_kind_default(&mut self, kind: MessageKind) {
        if self.display_defaults.get(kind).is_some() {
            let previous = self.display_defaults.clone();
            self.display_defaults.remove(kind);
            self.refresh_display_defaults(&previous, Some(kind), false);
        }
    }

    pub fn choose_kind_default(&mut self, kind: MessageKind, form: DisplayForm) {
        let previous = self.display_defaults.clone();
        self.display_defaults.set(kind, form);
        self.refresh_display_defaults(&previous, Some(kind), true);
    }

    pub fn replace_display_defaults(&mut self, defaults: DisplayDefaults, reset_overrides: bool) {
        if self.display_defaults == defaults && !reset_overrides {
            return;
        }
        let previous = std::mem::replace(&mut self.display_defaults, defaults);
        self.refresh_display_defaults(&previous, None, reset_overrides);
    }

    pub fn choose_kind_defaults(&mut self, kinds: &[MessageKind], form: DisplayForm) {
        let previous = self.display_defaults.clone();
        for &kind in kinds { self.display_defaults.set(kind, form); }
        self.refresh_kind_defaults(&previous, Some(kinds), true);
    }

    pub(super) fn refresh_display_defaults(
        &mut self,
        previous: &DisplayDefaults,
        target: Option<MessageKind>,
        reset_overrides: bool,
    ) {
        let kinds = target.map(|kind| [kind]);
        self.refresh_kind_defaults(previous, kinds.as_ref().map(|kinds| kinds.as_slice()), reset_overrides);
    }

    fn refresh_kind_defaults(
        &mut self,
        previous: &DisplayDefaults,
        targets: Option<&[MessageKind]>,
        reset_overrides: bool,
    ) {
        let edit_expanded = self.appearance.scrollback.blocks.edit.effective_expanded(
            crate::appearance::cache::load_collapsed_edit_blocks(),
        );
        let mut changed = false;
        for entry in self.entries.values_mut() {
            let kind = MessageKind::from(&entry.block);
            if targets.is_some_and(|targets| !targets.iter().any(|&target| target == kind || kind.parent() == Some(target))) {
                continue;
            }
            let form = self.display_defaults.get(kind);
            if !reset_overrides && previous.get(kind) == form {
                continue;
            }
            entry.form = EntryForm::FollowingKind;
            entry.display_mode_pinned = false;
            let mode = if form.is_none() && !entry.is_running {
                entry.block.finished_display_mode().unwrap_or_else(|| resolve_mode(&entry.block, form, edit_expanded))
            } else {
                resolve_mode(&entry.block, form, edit_expanded)
            };
            if entry.display_mode != mode {
                entry.display_mode = mode;
                entry.invalidate_cache();
                changed = true;
            }
        }
        if changed {
            self.invalidate_heights();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f7_choice_applies_to_producer_selected_modes_and_future_messages() {
        let mut state = ScrollbackState::default();
        state.choose_kind_default(MessageKind::AgentMessage, DisplayForm::Collapsed);
        let id = state.push(ScrollbackEntry::new(RenderBlock::agent_message("answer"))
            .with_display_mode(DisplayMode::Expanded));
        assert_eq!(state.get_by_id(id).unwrap().display_mode, DisplayMode::Collapsed);
        state.clear_kind_default(MessageKind::AgentMessage);
        assert_eq!(state.get_by_id(id).unwrap().display_mode, DisplayMode::Expanded);
    }

    #[test]
    fn changing_a_display_scope_reapplies_even_an_equal_saved_default() {
        let mut state = ScrollbackState::default();
        state.choose_kind_default(MessageKind::AgentMessage, DisplayForm::Collapsed);
        let id = state.push(ScrollbackEntry::new(RenderBlock::agent_message("answer")));
        let entry = state.get_by_id_mut(id).unwrap();
        entry.display_mode = DisplayMode::Expanded;
        entry.display_mode_pinned = true;
        state.replace_display_defaults(state.display_defaults().clone(), true);
        assert_eq!(state.get_by_id(id).unwrap().display_mode, DisplayMode::Collapsed);
        assert!(!state.get_by_id(id).unwrap().display_mode_pinned);
    }

    #[test]
    fn an_unrelated_global_display_change_preserves_a_manual_fold() {
        let mut state = ScrollbackState::default();
        let id = state.push(ScrollbackEntry::new(RenderBlock::tool_call("custom_tool", "args")));
        let entry = state.get_by_id_mut(id).unwrap();
        entry.display_mode = DisplayMode::Collapsed;
        entry.display_mode_pinned = true;
        let mut defaults = DisplayDefaults::default();
        defaults.set(MessageKind::AgentMessage, DisplayForm::Collapsed);
        state.replace_display_defaults(defaults, false);
        assert_eq!(state.get_by_id(id).unwrap().display_mode, DisplayMode::Collapsed);
        assert!(state.get_by_id(id).unwrap().display_mode_pinned);
    }

    #[test]
    fn f7_edit_choice_survives_refinement_finish_and_legacy_default_changes() {
        let mut state = ScrollbackState::default();
        state.choose_kind_default(MessageKind::Edit, DisplayForm::Collapsed);
        let mut pending = ScrollbackEntry::new(RenderBlock::tool_call("edit_file", "args"));
        pending.is_running = true;
        let id = state.push(pending);
        let edit = EditToolCallBlock::new("src/main.rs", vec![]).with_untrusted_summary();
        assert!(state.replace_tool_block(id, ToolCallBlock::Edit(edit)));
        state.finish_running(id);
        state.apply_collapsed_edit_blocks_flip(false, true);
        state.apply_collapsed_edit_blocks_flip(true, false);
        assert_eq!(state.get_by_id(id).unwrap().display_mode, DisplayMode::Collapsed);
    }
}
