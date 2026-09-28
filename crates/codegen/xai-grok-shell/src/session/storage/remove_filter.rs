use crate::session::ContextItemRef;
use crate::session::storage::{RawLinePeek, RawParamsPeek, RawUpdatePeek, SessionUpdate, XAI_SESSION_UPDATE_METHOD};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationRemoval {
    Removed(usize),
    NotFound,
    WouldOrphan,
}

pub(crate) fn update_matches_removed_item(item: &crate::session::storage::SessionUpdate, refs: &[ContextItemRef]) -> bool {
    match item {
        SessionUpdate::Acp(notification) => match &notification.update {
            agent_client_protocol::SessionUpdate::UserMessageChunk(chunk) => {
                let prompt_index = chunk.meta.as_ref().and_then(|meta| meta.get("promptIndex")).and_then(serde_json::Value::as_u64).and_then(|value| usize::try_from(value).ok());
                refs.iter().any(|reference| matches!(reference, ContextItemRef::Turn { prompt_index: removed } if Some(*removed) == prompt_index))
            }
            agent_client_protocol::SessionUpdate::ToolCall(call) => refs.iter().any(|reference| matches!(reference, ContextItemRef::ToolExchange { tool_call_id } if tool_call_id.as_str() == call.tool_call_id.0.as_ref())),
            agent_client_protocol::SessionUpdate::ToolCallUpdate(call) => refs.iter().any(|reference| matches!(reference, ContextItemRef::ToolExchange { tool_call_id } if tool_call_id.as_str() == call.tool_call_id.0.as_ref())),
            _ => false,
        },
        SessionUpdate::Xai(notification) => match &notification.update {
            crate::extensions::notification::SessionUpdate::ContextItemsRemoved { items, .. } => items.iter().any(|removed| refs.contains(removed)),
            _ => false,
        },
    }
}

pub(crate) fn raw_line_matches_removed_item(line: &str, refs: &[ContextItemRef]) -> bool {
    let Ok(envelope) = serde_json::from_str::<RawLinePeek<'_>>(line) else { return false };
    let (Some(method), Some(params)) = (envelope.method, envelope.params) else { return false };
    let Ok(params) = serde_json::from_str::<RawParamsPeek<'_>>(params.get()) else { return false };
    let Some(update) = params.update else { return false };
    if method == XAI_SESSION_UPDATE_METHOD {
        return update.session_update == *crate::session::wire_tags::CONTEXT_ITEMS_REMOVED
            && update.items.as_ref().is_some_and(|items| items.iter().any(|removed| refs.contains(removed)));
    }
    raw_update_matches_removed_item(&update, refs)
}

fn raw_update_matches_removed_item(update: &RawUpdatePeek<'_>, refs: &[ContextItemRef]) -> bool {
    match update.session_update {
        "user_message_chunk" => {
            let prompt_index = update.meta.as_ref().and_then(|meta| meta.prompt_index).and_then(|value| usize::try_from(value).ok());
            refs.iter().any(|reference| matches!(reference, ContextItemRef::Turn { prompt_index: removed } if Some(*removed) == prompt_index))
        }
        "tool_call" | "tool_call_update" => {
            let call_id = update.tool_call_id;
            refs.iter().any(|reference| matches!(reference, ContextItemRef::ToolExchange { tool_call_id: removed } if Some(removed.as_str()) == call_id))
        }
        _ => false,
    }
}

pub(crate) fn remove_from_conversation(
    items: &mut Vec<crate::sampling::ConversationItem>,
    refs: &[ContextItemRef],
) -> ConversationRemoval {
    use crate::sampling::ConversationItem;
    let original = items.len();
    let mut tool_ids = std::collections::HashSet::new();
    let mut found = false;
    let mut invalid_pair = false;
    let mut ranges = Vec::new();
    for reference in refs {
        match reference {
            ContextItemRef::Turn { prompt_index } => {
                let Some(start) = items.iter().position(|item| matches!(item, ConversationItem::User(user) if user.prompt_index == Some(*prompt_index))) else { continue };
                found = true;
                let end = items.iter().enumerate().skip(start + 1).find_map(|(idx, item)| match item {
                    ConversationItem::User(user) if user.prompt_index.is_some_and(|index| index > *prompt_index) => Some(idx),
                    _ => None,
                }).unwrap_or(items.len());
                for item in items.iter().take(end).skip(start) {
                    if let ConversationItem::Assistant(assistant) = item {
                        tool_ids.extend(assistant.tool_calls.iter().map(|call| call.id.as_ref()));
                    }
                }
                ranges.push(start..end);
            }
            ContextItemRef::ToolExchange { tool_call_id } => {
                let calls = items.iter().flat_map(|item| match item {
                    ConversationItem::Assistant(assistant) => assistant.tool_calls.iter().filter(|call| call.id.as_ref() == tool_call_id.as_str()).count(),
                    _ => 0,
                }).sum::<usize>();
                let results = items.iter().filter(|item| matches!(item, ConversationItem::ToolResult(result) if result.tool_call_id == tool_call_id.as_str())).count();
                match (calls, results) {
                    (0, 0) => {}
                    (1, 1) => {
                        tool_ids.insert(tool_call_id.as_str());
                        found = true;
                    }
                    _ => invalid_pair = true,
                }
            }
        }
    }
    if invalid_pair { return ConversationRemoval::WouldOrphan; }
    if !found { return ConversationRemoval::NotFound; }
    ranges.sort_by_key(|range| range.start);
    for range in ranges.into_iter().rev() { items.drain(range); }
    items.retain_mut(|item| match item {
        ConversationItem::ToolResult(result) => !tool_ids.contains(result.tool_call_id.as_str()),
        ConversationItem::Assistant(assistant) => {
            let original_calls = assistant.tool_calls.len();
            assistant.tool_calls.retain(|call| !tool_ids.contains(call.id.as_ref()));
            let removed_call = assistant.tool_calls.len() < original_calls;
            !(removed_call && assistant.content.is_empty() && assistant.tool_calls.is_empty())
        }
        _ => true,
    });
    ConversationRemoval::Removed(original.saturating_sub(items.len()))
}
