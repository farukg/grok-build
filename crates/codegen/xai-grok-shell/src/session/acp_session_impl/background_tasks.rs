//! Focused `SessionActor` helpers for durable `background_tasks` snapshots.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::*;
use crate::extensions::background_task::{
    SnapshotListOutcome, background_tasks_update, snapshot_list_outcome,
};
use crate::session::acp_types::{SESSION_ACTIVITY_PROVENANCE_META_KEY, SessionActivityProvenance};

/// Overall budget for snapshot `list_tasks_metadata` on the session actor.
/// Local ACP enumeration is sync, but other backends still go through an await;
/// a hung client must not stall every later `SessionCommand`.
pub(crate) const BACKGROUND_TASKS_LIST_BUDGET: Duration = Duration::from_secs(2);

impl SessionActor {
    /// List (metadata-only), fit, persist, and broadcast a last-wins
    /// `SessionUpdate::BackgroundTasks` snapshot.
    ///
    /// Timeout skips the emit (does not persist `tasks: []`). Missing backend
    /// still clears. Coalescing relies on a queued follow-up command rather than
    /// an in-arm retry loop.
    pub(super) async fn emit_background_tasks_snapshot(
        &self,
        provenance: SessionActivityProvenance,
        pending: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) {
        // Always clear coalescing pending so a skipped emit cannot stick true.
        if let Some(flag) = &pending {
            flag.store(false, Ordering::Release);
        }

        if !self.emit_local_background_tasks.load(Ordering::Acquire) {
            tracing::debug!(
                session_id = %self.session_info.id,
                "skipping local background_tasks snapshot emit (gateway-backed)"
            );
            return;
        }

        let list_result = tokio::time::timeout(
            BACKGROUND_TASKS_LIST_BUDGET,
            self.tool_bridge_handle().list_tasks_metadata(),
        )
        .await
        .map_err(|_| ());

        // Re-check after the list await: a mid-session gateway bind can flip the
        // flag off while we were listing; broadcasting then would last-wins-clear
        // remote Running — the case the flag was added to prevent.
        if !self.emit_local_background_tasks.load(Ordering::Acquire) {
            tracing::debug!(
                session_id = %self.session_info.id,
                "skipping local background_tasks snapshot emit after list (gateway-backed)"
            );
            return;
        }

        match snapshot_list_outcome(list_result) {
            SnapshotListOutcome::SkipTimeout => {
                tracing::warn!(
                    session_id = %self.session_info.id,
                    budget_ms = BACKGROUND_TASKS_LIST_BUDGET.as_millis() as u64,
                    "background_tasks list_tasks exceeded budget; skipping snapshot emit"
                );
            }
            SnapshotListOutcome::ClearMissingBackend => {
                tracing::warn!(
                    session_id = %self.session_info.id,
                    "emitting empty background_tasks snapshot: no terminal backend"
                );
                self.broadcast_background_tasks_update(std::iter::empty(), provenance)
                    .await;
            }
            SnapshotListOutcome::Tasks(snapshots) => {
                self.broadcast_background_tasks_update(snapshots, provenance)
                    .await;
            }
        }
    }

    async fn broadcast_background_tasks_update(
        &self,
        snapshots: impl IntoIterator<Item = xai_grok_tools::types::TaskSnapshot>,
        provenance: SessionActivityProvenance,
    ) {
        let mut extra_meta = serde_json::Map::new();
        extra_meta.insert(
            SESSION_ACTIVITY_PROVENANCE_META_KEY.to_owned(),
            json!(provenance),
        );
        let mut meta = self.build_notification_meta();
        if let serde_json::Value::Object(fields) = &mut meta {
            fields.extend(extra_meta.clone());
        }
        let Some(update) = background_tasks_update(snapshots, &self.session_info.id, Some(meta))
        else {
            tracing::warn!(
                session_id = %self.session_info.id,
                "skipping unfittable background_tasks snapshot"
            );
            return;
        };
        if let Err(error) = self
            .send_xai_notification_with_extra_meta(
                update,
                Some(extra_meta),
                crate::session::storage::jsonl::AppendDurability::Buffered,
            )
            .await
        {
            tracing::error!(?error, "background_tasks snapshot persistence unavailable");
        }
    }
}

#[cfg(test)]
#[path = "../acp_session_tests/background_activity_tests.rs"]
mod tests;
