use super::ctx::get_active_agent_mut;
use crate::acp::turn_ledger::TurnLedger;
use crate::app::actions::Effect;
use crate::app::agent_view::AgentView;
use crate::app::agent_view::removal_verdict::items_phrase;
use crate::app::app_view::AppView;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::entry::EntryId;
use crate::scrollback::state::ScrollbackState;
use std::collections::{HashMap, HashSet};
use xai_grok_shell::session::{
    ContextItemRef, RemoveContextItemsOutcome, SkipReason, SkippedItem, ToolCallId,
};

/// Asks the shell to drop `items` from the active session's model context.
/// One removal per session may be in flight: the answer carries no request id, so it is paired with the request by session.
pub(super) fn dispatch_remove_context_items(
    app: &mut AppView,
    items: Vec<ContextItemRef>,
) -> Vec<Effect> {
    let Some(agent) = get_active_agent_mut(app) else {
        return vec![];
    };
    let Some(session_id) = agent.session.session_id.clone() else {
        agent.show_toast("Session is not connected");
        return vec![];
    };
    if agent.pending_removed_context_items.contains(&session_id) {
        agent.show_toast("A removal is already in progress");
        return vec![];
    }
    agent.pending_removed_context_items.push(session_id.clone());
    vec![Effect::RemoveContextItems { session_id, items }]
}

/// A turn is its prompt up to the next prompt that starts a shell turn; interjections and `!` commands belong to the turn they interrupt.
fn starts_turn(block: &RenderBlock) -> bool {
    matches!(block, RenderBlock::UserPrompt(prompt) if !prompt.is_interjection && !prompt.is_bash)
}

/// The rows of one shell turn: from its prompt to the entry before the next prompt, or through the entry a hidden (auto-wake) turn follows.
fn turn_entries(
    scrollback: &ScrollbackState,
    ledger: &TurnLedger,
    prompt_index: usize,
) -> Vec<EntryId> {
    let Some(start) = (0..scrollback.len()).find(|&idx| {
        scrollback.get(idx).is_some_and(|entry| {
            matches!(&entry.block, RenderBlock::UserPrompt(prompt)
                if !prompt.is_interjection && prompt.prompt_index == Some(prompt_index))
        })
    }) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for idx in start..scrollback.len() {
        let Some(entry) = scrollback.get(idx) else {
            break;
        };
        if idx > start && starts_turn(&entry.block) {
            break;
        }
        entries.push(entry.id);
        if ledger.turn_ends_after(entry.id) {
            break;
        }
    }
    entries
}

/// The entries that stand for what the shell reports as removed.
fn removed_entries(
    scrollback: &ScrollbackState,
    ledger: &TurnLedger,
    tool_ids: &HashMap<EntryId, ToolCallId>,
    removed: &[ContextItemRef],
) -> HashSet<EntryId> {
    let mut entries = HashSet::new();
    for item in removed {
        match item {
            ContextItemRef::Turn { prompt_index } => {
                entries.extend(turn_entries(scrollback, ledger, *prompt_index));
            }
            ContextItemRef::ToolExchange { tool_call_id } => {
                entries.extend(scrollback.iter_entries().filter_map(|(id, entry)| {
                    (matches!(entry.block, RenderBlock::ToolCall(_))
                        && tool_ids.get(&id) == Some(tool_call_id))
                    .then_some(id)
                }));
            }
        }
    }
    entries
}

/// Takes the rows for `removed` out of the scrollback and the tracker, keeping turn boundaries for the rows that stay.
fn drop_removed_rows(view: &mut AgentView, removed: &[ContextItemRef]) {
    let tool_ids = view.session.tracker.tool_call_ids_by_entry();
    let rows = removed_entries(
        &view.scrollback,
        &view.session.tracker.turn_ledger,
        &tool_ids,
        removed,
    );
    view.session
        .tracker
        .turn_ledger
        .carry_to_survivors(&view.scrollback, &rows);
    let row_ids: Vec<EntryId> = rows.into_iter().collect();
    for id in &row_ids {
        view.scrollback.remove_entry(*id);
    }
    view.session.tracker.remove_context_tool_call_ids(&row_ids);
}

fn skip_reason_text(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::NotInContext => "not in the model context",
        SkipReason::CallWithoutResult => "tool call has no result yet",
        SkipReason::AmbiguousId => "ambiguous id",
    }
}

fn removed_toast(removed: usize, skipped: &[SkippedItem]) -> String {
    let base = format!("Removed {} from the model context", items_phrase(removed));
    match skipped.first() {
        Some(first) => format!(
            "{base}; {} skipped: {}",
            skipped.len(),
            skip_reason_text(first.reason)
        ),
        None => base,
    }
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
        tracing::warn!(
            "Dropped remove-items answer for unknown session {}",
            session_id.0
        );
        return;
    };
    let Some(position) = view
        .pending_removed_context_items
        .iter()
        .position(|pending_session| pending_session == session_id)
    else {
        tracing::warn!(
            "Dropped remove-items answer without a pending request for session {}",
            session_id.0
        );
        return;
    };
    view.pending_removed_context_items.remove(position);
    let toast = match outcome {
        Ok(RemoveContextItemsOutcome::Removed { removed, skipped }) => {
            drop_removed_rows(view, &removed);
            removed_toast(removed.len(), &skipped)
        }
        Ok(RemoveContextItemsOutcome::TurnRunning) => {
            "Wait until the running turn finishes, then try again".to_owned()
        }
        Ok(RemoveContextItemsOutcome::NotFound) => {
            "The model context no longer contains this item".to_owned()
        }
        Ok(RemoveContextItemsOutcome::BeforeCompaction) => {
            "This turn was compacted into the summary and cannot be removed".to_owned()
        }
        Ok(RemoveContextItemsOutcome::WouldOrphan) => {
            "Removing this item would orphan a tool result".to_owned()
        }
        Ok(RemoveContextItemsOutcome::PersistFailed { removed }) => {
            drop_removed_rows(view, &removed);
            "Removed in memory but could not be saved; it may return after a restart".to_owned()
        }
        Err(error) => format!("Could not remove from the model context: {error}"),
    };
    view.show_toast(&toast);
}

#[cfg(test)]
#[path = "rewind_remove_tests.rs"]
mod tests;
