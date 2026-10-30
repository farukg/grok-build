use super::*;

/// Update the activity label on a subagent's collapsed scrollback block.
/// Skips the write (and cache invalidation) when the label hasn't changed.
/// Most deltas keep the same label ("Responding" stays "Responding"), so the common case allocates nothing.
pub(super) fn sync_activity_label(
    scrollback: &mut crate::scrollback::state::ScrollbackState,
    entry_id: Option<crate::scrollback::entry::EntryId>,
    activity_label: Option<&str>,
) {
    if let Some(eid) = entry_id
        && let Some(entry) = scrollback.get_by_id_mut(eid)
        && let RenderBlock::Subagent(ref mut sb) = entry.block
        && sb.activity_label.as_deref() != activity_label
    {
        sb.activity_label = activity_label.map(str::to_owned);
        entry.invalidate_cache();
    }
}

/// Fan a subagent's computed activity label out to both places that show it, so the two can't drift.
/// Those are the collapsed scrollback block and the [`SubagentInfo`] backing the tasks pane and dashboard rows.
/// Once `finished` is set, only a clear (`None`) lands: buffered updates from the child race `SubagentFinished` and must not re-stamp the label.
pub(super) fn sync_subagent_activity(
    parent: &mut AgentView,
    child_key: &str,
    activity_label: Option<String>,
) {
    let Some(info) = parent.subagent_sessions.get_mut(child_key) else {
        return;
    };
    if info.is_finished() && activity_label.is_some() {
        return;
    }
    sync_activity_label(
        &mut parent.scrollback,
        info.attempt.scrollback_entry_id,
        activity_label.as_deref(),
    );
    info.attempt.activity_label = activity_label;
}

/// Resolve a subagent child view's live activity into the display label [`sync_subagent_activity`] stamps.
/// A child that is busy between activities shows "Waiting".
pub(super) fn subagent_activity_label(child_view: &AgentView) -> Option<String> {
    match child_view.resolve_turn_activity() {
        Some(a) => Some(crate::app::subagent::format_activity_label(&a)),
        None if child_view.session.state.is_busy() => Some("Waiting".to_string()),
        None => None,
    }
}

