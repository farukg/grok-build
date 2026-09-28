use std::collections::HashSet;
use std::sync::Arc;

use crate::session::ContextItemRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConversationRemoval {
    Removed(usize),
    NotFound,
    WouldOrphan,
}

pub(crate) fn remove_from_conversation(
    items: &mut Vec<crate::sampling::ConversationItem>,
    refs: &[ContextItemRef],
) -> ConversationRemoval {
    use crate::sampling::ConversationItem;
    let original = items.len();
    let mut tool_ids: HashSet<Arc<str>> = HashSet::new();
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
                        tool_ids.extend(assistant.tool_calls.iter().map(|call| call.id.clone()));
                    }
                }
                ranges.push(start..end);
            }
            ContextItemRef::ToolExchange { tool_call_id } => {
                let calls = items.iter().map(|item| match item {
                    ConversationItem::Assistant(assistant) => assistant.tool_calls.iter().filter(|call| call.id.as_ref() == tool_call_id.as_str()).count(),
                    _ => 0,
                }).sum::<usize>();
                let results = items.iter().filter(|item| matches!(item, ConversationItem::ToolResult(result) if result.tool_call_id == tool_call_id.as_str())).count();
                match (calls, results) {
                    (0, 0) => {}
                    (1, 1) => {
                        tool_ids.insert(Arc::from(tool_call_id.as_str()));
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
    ranges.dedup();
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
