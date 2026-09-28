use super::*;
use crate::session::{ContextItemRef, RemoveContextItemsOutcome, RemoveItemsRequest};
use crate::session::persistence::PersistenceMsg;
use crate::extensions::notification::SessionUpdate as XaiSessionUpdate;

impl SessionActor {
    pub(super) async fn handle_remove_context_items(
        &self,
        request: RemoveItemsRequest,
    ) -> RemoveContextItemsOutcome {
        if self.session_turn_active.load(std::sync::atomic::Ordering::SeqCst) {
            return RemoveContextItemsOutcome::TurnRunning;
        }
        let Some(snapshot) = self.chat_state_handle.snapshot().await else {
            return RemoveContextItemsOutcome::NotFound;
        };
        if let Some(compaction_at) = snapshot.last_compaction_prompt_index
            && request.items.iter().any(|item| matches!(item, ContextItemRef::Turn { prompt_index } if *prompt_index < compaction_at))
        {
            return RemoveContextItemsOutcome::BeforeCompaction;
        }
        let mut conversation = snapshot.conversation;
        let mut remove_turn_starts = request.items.iter().filter_map(|item| match item {
            ContextItemRef::Turn { prompt_index } => Some(*prompt_index),
            ContextItemRef::ToolExchange { .. } => None,
        }).collect::<Vec<_>>();
        remove_turn_starts.sort_unstable();
        if remove_turn_starts.windows(2).any(|pair| pair[0] == pair[1]) {
            return RemoveContextItemsOutcome::NotFound;
        }
        let removed_refs = request.items.iter().filter(|item| match item {
            ContextItemRef::Turn { prompt_index } => conversation.iter().any(|message| matches!(message,
                crate::sampling::ConversationItem::User(user) if user.prompt_index == Some(*prompt_index))),
            ContextItemRef::ToolExchange { tool_call_id } => {
                let calls = conversation.iter().filter(|message| matches!(message,
                    crate::sampling::ConversationItem::Assistant(assistant) if assistant.tool_calls.iter().any(|call| call.id == tool_call_id.as_str()))).count();
                let results = conversation.iter().filter(|message| matches!(message,
                    crate::sampling::ConversationItem::ToolResult(result) if result.tool_call_id == tool_call_id.as_str())).count();
                calls == 1 && results == 1
            }
        }).cloned().collect::<Vec<_>>();
        if removed_refs.is_empty() { return RemoveContextItemsOutcome::NotFound; }
        match crate::session::storage::remove_filter::remove_from_conversation(&mut conversation, &removed_refs) {
            crate::session::storage::remove_filter::ConversationRemoval::Removed(removed) => {
                if self.session_turn_active.load(std::sync::atomic::Ordering::SeqCst) {
                    return RemoveContextItemsOutcome::TurnRunning;
                }
                self.chat_state_handle.replace_conversation(conversation);
                let (flush_tx, flush_rx) = tokio::sync::oneshot::channel();
                if self.notifications.persistence_tx.send(PersistenceMsg::FlushAndAck { respond_to: flush_tx }).is_err()
                    || !matches!(flush_rx.await, Ok(Ok(())))
                {
                    return RemoveContextItemsOutcome::TurnRunning;
                }
                self.persist_xai_update_only(XaiSessionUpdate::ContextItemsRemoved {
                    items: removed_refs,
                    created_at: chrono::Utc::now().to_rfc3339(),
                });
                let (marker_tx, marker_rx) = tokio::sync::oneshot::channel();
                if self.notifications.persistence_tx.send(PersistenceMsg::FlushAndAck { respond_to: marker_tx }).is_err()
                    || !matches!(marker_rx.await, Ok(Ok(())))
                {
                    return RemoveContextItemsOutcome::TurnRunning;
                }
                RemoveContextItemsOutcome::Removed { items_removed: removed }
            }
            crate::session::storage::remove_filter::ConversationRemoval::NotFound => RemoveContextItemsOutcome::NotFound,
            crate::session::storage::remove_filter::ConversationRemoval::WouldOrphan => RemoveContextItemsOutcome::WouldOrphan,
        }
    }
}
