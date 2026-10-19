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
        let removal = crate::session::storage::remove_filter::remove_from_conversation(&mut conversation, &request.items);
        if removal.removed.is_empty() { return removal.refusal(); }
        if self.session_turn_active.load(std::sync::atomic::Ordering::SeqCst) {
            return RemoveContextItemsOutcome::TurnRunning;
        }
        self.chat_state_handle.replace_conversation(conversation);
        if !self.removal_is_durable(&removal.removed).await {
            return RemoveContextItemsOutcome::PersistFailed { removed: removal.removed };
        }
        RemoveContextItemsOutcome::Removed { removed: removal.removed, skipped: removal.skipped }
    }

    /// The history rewrite is acknowledged before the replay marker is written, so a marker never outlives a rewrite that did not land.
    async fn removal_is_durable(&self, removed: &[ContextItemRef]) -> bool {
        // Chat-state commands run in order: this reply proves the replace has run and queued its rewrite ahead of the barrier.
        if self.chat_state_handle.try_get_estimated_total_tokens().await.is_none() {
            return false;
        }
        if !self.persistence_barrier().await {
            return false;
        }
        self.persist_xai_update_only(XaiSessionUpdate::ContextItemsRemoved {
            items: removed.to_vec(),
            created_at: chrono::Utc::now().to_rfc3339(),
        });
        self.persistence_barrier().await
    }

    async fn persistence_barrier(&self) -> bool {
        let (flush_tx, flush_rx) = tokio::sync::oneshot::channel();
        self.notifications.persistence_tx.send(PersistenceMsg::FlushAndAck { respond_to: flush_tx }).is_ok()
            && matches!(flush_rx.await, Ok(Ok(())))
    }
}
