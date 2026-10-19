//! Setting the expansion of chosen fold groups directly, without going through the selection.

use super::*;

impl ScrollbackState {
    /// Set each group keyed by its first entry's id (`true` = expanded) with a single layout rebuild.
    /// `anchor_idx` is the entry whose screen row the rebuild keeps in place.
    /// Returns whether any group changed. Same rebuild, anchoring and selection repair as the chat's own group toggle.
    pub fn set_group_expansion(&mut self, changes: &[(EntryId, bool)], anchor_idx: usize) -> bool {
        let anchor = self.capture_fold_anchor(anchor_idx);
        let revealed = changes
            .iter()
            .filter(|&&(first_id, expand)| expand && !self.expanded_groups.contains(&first_id))
            .find_map(|&(first_id, _)| {
                let span = self.span_at(self.index_of_id(first_id)?)?;
                Some(span.range.clone())
            });
        let mut changed = false;
        let mut deselect = false;
        for &(first_id, expand) in changes {
            if expand {
                if self.expanded_groups.insert(first_id) {
                    changed = true;
                    deselect |= self.selection_in_truncation_run(first_id);
                }
            } else {
                changed |= self.expanded_groups.remove(&first_id);
            }
        }
        if !changed {
            return false;
        }
        let change = match revealed {
            Some(block) => FoldChange::Expand { block },
            None => FoldChange::Collapse,
        };
        self.rebuild_with_fold_anchor(anchor_idx, change, anchor);
        self.fixup_hidden_selection();
        // Opening an "N more" run: the selected first entry would otherwise look active beside the collapse header
        if deselect {
            self.selected = None;
        }
        self.bump_generation();
        true
    }

    /// Whether the selection sits in the "N more" run starting at `first_id`.
    fn selection_in_truncation_run(&self, first_id: EntryId) -> bool {
        let Some(selected) = self.selected else {
            return false;
        };
        self.index_of_id(first_id)
            .and_then(|idx| self.span_at(idx))
            .is_some_and(|span| {
                matches!(span.kind, groups::GroupKind::Truncation { .. })
                    && span.range.contains(&selected)
            })
    }
}
