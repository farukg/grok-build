use super::*;

impl SessionActor {
    pub(in crate::session::acp_session) async fn admit_task_completion_batch(
        &self,
        batch: crate::session::commands::TaskCompletionBatch,
    ) {
        let crate::session::commands::TaskCompletionBatch {
            prompt_id,
            automatic,
            admissions,
            respond_to,
        } = batch;
        let persisted = match self.persisted_completion_state().await {
            Ok(persisted) => persisted,
            Err(error) => {
                tracing::error!(%error, "terminal publication retained until receipts can be read");
                let mut state = self.state.lock().await;
                for admission in admissions {
                    let task_id = admission.fallback.source.task_id();
                    if state
                        .pending_notifications
                        .iter()
                        .any(|row| row.source.task_id() == task_id)
                    {
                        if let Some(reservations) = &self.tool_context.task_completion_reservations
                        {
                            reservations.release(task_id);
                        }
                    } else {
                        Self::push_task_wake_fallback(&mut state, admission.fallback);
                    }
                    let _ = admission.respond_to.send(false);
                }
                Self::respond_removed_prompt(respond_to);
                return;
            }
        };
        let mut notifications = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for admission in admissions {
            let notification = PendingNotification {
                prompt_id: admission.fallback.prompt_id.clone(),
                prompt_blocks: admission.fallback.prompt_blocks.clone(),
                source: admission.fallback.source.clone(),
                priority: NotificationPriority::Later,
            };
            let ids = self.notification_completion_ids(std::slice::from_ref(&notification));
            let consumed = ids.iter().any(|id| {
                persisted
                    .receipts
                    .iter()
                    .any(|receipt| &receipt.completion == id)
                    || persisted.obligations.iter().any(|obligation| {
                        matches!(obligation,
                        CompletionObligation::Suppressed { id: suppressed, .. } if suppressed == id)
                    })
            });
            let task_id = admission.fallback.source.task_id().to_owned();
            let duplicate = {
                let state = self.state.lock().await;
                state
                    .pending_notifications
                    .iter()
                    .any(|row| row.source.task_id() == task_id)
                    || state.pending_inputs.iter().any(|input| {
                        input.input_origin.completion_id() == Some(task_id.as_str())
                            || input
                                .input_origin
                                .completions()
                                .iter()
                                .any(|id| ids.contains(id))
                    })
                    || !seen.insert(task_id.clone())
            };
            if consumed || duplicate {
                if let Some(reservations) = &self.tool_context.task_completion_reservations {
                    reservations.release(&task_id);
                }
                let _ = admission.respond_to.send(false);
                continue;
            }
            if !automatic {
                Self::push_task_wake_fallback(&mut self.state.lock().await, admission.fallback);
                let _ = admission.respond_to.send(false);
                continue;
            }
            let origin = PromptOrigin::TaskCompleted { task_id };
            if let Some(fallback) = self.admit_task_completion_wake(&origin, admission).await {
                notifications.push(PendingNotification {
                    prompt_id: fallback.prompt_id,
                    prompt_blocks: fallback.prompt_blocks,
                    priority: NotificationPriority::Later,
                    source: fallback.source,
                });
            }
        }
        if !notifications.is_empty() {
            self.ensure_prefix_ready().await;
            self.observe_wait_input(&PromptOrigin::NotificationDrain);
        }
        let mut state = self.state.lock().await;
        if self.queue_notification_batch(&mut state, notifications)
            && let Some(input) = state.pending_inputs.back_mut()
        {
            input.prompt_id = prompt_id;
            input.respond_to = respond_to;
        } else {
            Self::respond_removed_prompt(respond_to);
        }
    }
}

#[cfg(test)]
#[path = "task_completion_publication_tests.rs"]
mod tests;
