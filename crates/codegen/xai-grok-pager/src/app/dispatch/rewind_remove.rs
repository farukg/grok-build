use crate::app::app_view::AppView;
use crate::scrollback::entry::EntryId;
use crate::scrollback::state::ScrollbackState;
use xai_grok_shell::session::ContextItemRef;
use xai_grok_shell::session::RemoveContextItemsOutcome;

fn find_removed_entries(
    scrollback: &ScrollbackState,
    refs: &[ContextItemRef],
    tool_id_for_entry: impl Fn(EntryId) -> Option<xai_grok_shell::session::ToolCallId>,
) -> Vec<EntryId> {
    scrollback.iter_entries().filter_map(|(id, entry)| {
        let removed = refs.iter().any(|reference| match reference {
            ContextItemRef::Turn { prompt_index } => matches!(&entry.block,
                crate::scrollback::block::RenderBlock::UserPrompt(prompt) if prompt.prompt_index == Some(*prompt_index)),
            ContextItemRef::ToolExchange { tool_call_id } => matches!(&entry.block,
                crate::scrollback::block::RenderBlock::ToolCall(_) if tool_id_for_entry(id).as_ref() == Some(tool_call_id)),
        });
        removed.then_some(id)
    }).collect()
}

pub(super) fn handle_remove_context_items_complete(
    app: &mut AppView,
    session_id: &agent_client_protocol::SessionId,
    outcome: Result<RemoveContextItemsOutcome, String>,
) {
    let Some((_, view)) = app
        .agents
        .all_mut()
        .find(|(_, view)| view.session.session_id.as_ref() == Some(session_id))
    else {
        return;
    };
    let Some(position) = view
        .pending_removed_context_items
        .iter()
        .position(|(pending_session, _)| pending_session == session_id)
    else {
        return;
    };
    let (_, refs) = view.pending_removed_context_items.remove(position);
    match outcome {
        Ok(RemoveContextItemsOutcome::Removed { .. }) => {
            let tool_ids = view.session.tracker.tool_call_ids_by_entry();
            let removed_ids =
                find_removed_entries(&view.scrollback, &refs, |id| tool_ids.get(&id).cloned());
            for id in &removed_ids {
                view.scrollback.remove_entry(*id);
            }
            view.session.tracker.remove_context_tool_call_ids(&removed_ids);
            view.show_toast("Removed from model context");
        }
        Ok(RemoveContextItemsOutcome::TurnRunning) => {
            view.show_toast("Cannot remove context while a turn is running");
        }
        Ok(RemoveContextItemsOutcome::NotFound) => view.show_toast("Context item was not found"),
        Ok(RemoveContextItemsOutcome::BeforeCompaction) => {
            view.show_toast("Context item is before compaction and cannot be removed");
        }
        Ok(RemoveContextItemsOutcome::WouldOrphan) => {
            view.show_toast("Removing this item would orphan a tool result");
        }
        Err(error) => view.show_toast(&format!("Could not remove from model context: {error}")),
    }
}
