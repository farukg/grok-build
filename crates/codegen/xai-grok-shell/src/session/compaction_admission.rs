//! Ordered manual compaction admission at conversation boundaries.

use super::super::*;
use crate::session::compaction_config::{ManualCompactWorker, PendingCompact};

impl SessionActor {
    fn compact_predecessors(&self, state: &State) -> std::collections::HashSet<String> {
        let wake = CompletionWakeAdmission::classify(
            self.tool_context.task_wake_suppressed.as_ref().is_some_and(|gate| gate.get()),
            state.notifications_suppressed,
        );
        state.pending_inputs.iter().filter(|input| {
            !input.input_origin.is_boundary_completion()
                && (!wake.is_parked() || input.input_origin.policy().authority.is_human_intent())
        }).map(|input| input.prompt_id.clone()).collect()
    }

    pub(in crate::session::acp_session) async fn enqueue_manual_compact(
        &self,
        invocation_id: Option<crate::session::acp_types::CompactInvocationId>,
        cancellation: tokio_util::sync::CancellationToken,
        respond_to: oneshot::Sender<acp::Result<()>>,
    ) {
        if self.compaction.cancel.manual_is_sealed() {
            let _ = respond_to.send(Err(crate::session::helpers::session_compact::CompactFailure::cancelled_error()));
            return;
        }
        let state = self.state.lock().await;
        let predecessors = self.compact_predecessors(&state);
        let queued = state.running_task.is_some()
            || state.finalization_gate.is_active()
            || !state.pending_inputs.is_empty()
            || !self.compaction.cancel.pending_manual.borrow().is_empty();
        self.compaction.cancel.pending_manual.borrow_mut().push_back(PendingCompact {
            id: uuid::Uuid::new_v4(),
            invocation_id,
            predecessors,
            cancellation,
            respond_to,
        });
        drop(state);
        if queued {
            let tokens_used = self.chat_state_handle.get_total_tokens().await;
            let context_window = self.chat_state_handle.get_sampling_config().await
                .map(|config| config.context_window.get())
                .unwrap_or(crate::remote::DEFAULT_CONTEXT_WINDOW);
            self.send_manual_compact_notification(invocation_id, crate::extensions::notification::SessionUpdate::AutoCompactStarted {
                phase: crate::extensions::notification::CompactPhase::Queued,
                tokens_used,
                context_window,
                percentage: xai_token_estimation::usage_percentage_u8(tokens_used, context_window),
                reason: "Compaction queued; waiting for the preceding conversation boundary. It will run automatically.".into(),
            }).await;
        }
    }

    pub(in crate::session::acp_session) fn pending_compact_allows_input(&self, prompt_id: &str) -> bool {
        self.compaction.cancel.pending_manual.borrow().front()
            .is_none_or(|request| request.predecessors.contains(prompt_id))
    }

    pub(in crate::session::acp_session) fn has_pending_compact(&self) -> bool {
        !self.compaction.cancel.pending_manual.borrow().is_empty()
    }

    pub(in crate::session::acp_session) fn start_pending_compact(
        self: &Arc<Self>,
        state: &mut State,
    ) -> bool {
        if !self.compaction.cancel.manual_can_start()
            || state.running_task.is_some()
            || state.finalization_gate.is_active()
            || !self.has_pending_compact()
        {
            return false;
        }
        let predecessors = self.compact_predecessors(state);
        let ready = self.compaction.cancel.pending_manual.borrow().front().is_some_and(|request| {
            request.predecessors.is_disjoint(&predecessors)
        });
        if !ready {
            return false;
        }
        let Some(mut finalization) = state.claim_cancel_finalization() else {
            return false;
        };
        // Keep the reserved request cancellable until its execution task takes the token scope.
        let Some(id) = self.compaction.cancel.pending_manual.borrow().front().map(|request| request.id) else {
            state.finish_finalization(&mut finalization);
            return false;
        };
        let session = self.clone();
        let worker = tokio::task::spawn_local(async move {
            #[cfg(test)]
            {
                let pause = session.compaction.cancel.manual_reservation_pause.borrow_mut().take();
                if let Some((reached, release)) = pause {
                    let _ = reached.send(());
                    let _ = release.await;
                }
            }
            let Some(request) = session.remove_pending_compact(id) else {
                session.state.lock().await.finish_finalization(&mut finalization);
                session.compaction.cancel.manual_finished.notify_one();
                return;
            };
            let (_cancel, scope) = session.compaction.cancel.enter();
            let result = {
                let compact = session.run_compact_with_invocation(request.invocation_id);
                tokio::pin!(compact);
                tokio::select! {
                    biased;
                    () = request.cancellation.cancelled() => {
                        session.compaction.cancel.request_cancel();
                        if session.compaction.cancel.is_committing() {
                            compact.await
                        } else {
                            Err(crate::session::helpers::session_compact::CompactFailure::cancelled_error())
                        }
                    }
                    result = &mut compact => result,
                }
            };
            let result = match result {
                Ok(()) => session.acknowledge_compaction_persistence().await,
                Err(error) => Err(error),
            };
            if let Err(error) = &result {
                session.emit_manual_compact_failure(request.invocation_id, error).await;
            }
            session.state.lock().await.finish_finalization(&mut finalization);
            let _ = request.respond_to.send(result);
            drop(scope);
            session.compaction.cancel.manual_finished.notify_one();
        });
        self.compaction.cancel.manual_worker.replace(ManualCompactWorker::Running(worker));
        true
    }

    pub(in crate::session::acp_session) async fn join_manual_compact_worker(&self) {
        let worker = self.compaction.cancel.take_manual_worker();
        if let Some(worker) = worker {
            if let Err(error) = worker.await {
                tracing::error!(%error, "manual compaction worker did not finish normally");
            }
        }
    }

    pub(in crate::session::acp_session) async fn shutdown_compaction(&self) {
        self.compaction.cancel.seal_manual();
        self.compaction.cancel.request_cancel();
        #[cfg(test)]
        self.compaction.cancel.manual_teardown_started.notify_one();
        self.cancel_pending_compactions().await;
        self.join_manual_compact_worker().await;
        self.compaction.cancel.drain().await;
    }

    pub(in crate::session::acp_session) async fn pending_compact_cancellation(&self) -> uuid::Uuid {
        let cancellations: Vec<_> = self.compaction.cancel.pending_manual.borrow().iter()
            .map(|request| {
                let id = request.id;
                let cancellation = request.cancellation.clone();
                Box::pin(async move {
                    cancellation.cancelled().await;
                    id
                })
            }).collect();
        if cancellations.is_empty() {
            std::future::pending().await
        } else {
            futures::future::select_all(cancellations).await.0
        }
    }

    fn remove_pending_compact(&self, id: uuid::Uuid) -> Option<PendingCompact> {
        let mut pending = self.compaction.cancel.pending_manual.borrow_mut();
        let index = pending.iter().position(|request| request.id == id)?;
        pending.remove(index)
    }

    pub(in crate::session::acp_session) async fn cancel_pending_compact(&self, id: uuid::Uuid) {
        if let Some(request) = self.remove_pending_compact(id) {
            let error = crate::session::helpers::session_compact::CompactFailure::cancelled_error();
            self.emit_manual_compact_failure(request.invocation_id, &error).await;
            let _ = request.respond_to.send(Err(error));
        }
    }

    pub(in crate::session::acp_session) async fn cancel_pending_compactions(&self) {
        let requests: Vec<_> = self.compaction.cancel.pending_manual.borrow_mut().drain(..).collect();
        for request in requests {
            let error = crate::session::helpers::session_compact::CompactFailure::cancelled_error();
            self.emit_manual_compact_failure(request.invocation_id, &error).await;
            let _ = request.respond_to.send(Err(error));
        }
    }
}
