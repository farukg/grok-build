//! Scrollback fixtures shared by the context-removal tests.

use super::AgentView;
use crate::acp::meta::NotificationMeta;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::UserPromptBlock;
use crate::scrollback::entry::EntryId;
use agent_client_protocol as acp;
use std::sync::Arc;

/// A user prompt the shell has already numbered.
pub(crate) fn push_numbered_prompt(
    agent: &mut AgentView,
    text: &str,
    prompt_index: usize,
) -> EntryId {
    let mut block = UserPromptBlock::new(text);
    block.prompt_index = Some(prompt_index);
    agent.scrollback.push_block(RenderBlock::UserPrompt(block))
}

/// A completed read tool call fed through the tracker, so its context id is registered like in a live session.
pub(crate) fn push_registered_read(agent: &mut AgentView, call_id: &str, path: &str) -> EntryId {
    let call = acp::ToolCall::new(acp::ToolCallId::new(Arc::from(call_id)), path.to_string())
        .kind(acp::ToolKind::Read)
        .status(acp::ToolCallStatus::Completed)
        .raw_input(Some(serde_json::json!({ "path": path })))
        .content(vec![])
        .locations(vec![]);
    agent.session.tracker.handle_update(
        acp::SessionUpdate::ToolCall(call),
        &NotificationMeta::default(),
        &mut agent.scrollback,
    );
    agent
        .scrollback
        .iter_entries()
        .last()
        .map(|(id, _)| id)
        .expect("the tracker pushed a tool entry")
}

/// An auto-wake prompt: the shell numbers it, the scrollback shows no prompt block for it.
pub(crate) fn feed_hidden_wake_prompt(agent: &mut AgentView, prompt_index: usize) {
    let mut meta = serde_json::Map::new();
    meta.insert("promptIndex".to_owned(), serde_json::json!(prompt_index));
    meta.insert("hideFromScrollback".to_owned(), serde_json::json!(true));
    let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(
        "background task finished".to_owned(),
    )))
    .meta(Some(meta));
    agent.session.tracker.handle_update(
        acp::SessionUpdate::UserMessageChunk(chunk),
        &NotificationMeta::default(),
        &mut agent.scrollback,
    );
}
