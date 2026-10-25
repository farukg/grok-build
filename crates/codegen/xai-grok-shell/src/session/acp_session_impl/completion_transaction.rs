use super::*;
use xai_chat_state::completion::{CompletionIdentity, TerminalCompletionId};
use crate::session::completion_delivery::{CompletionEventPolicy, CompletionObligation, CompletionOffer};

impl SessionActor {
    pub(crate) async fn suppress_terminal_notifications(&self, ids: Vec<CompletionIdentity>) -> std::io::Result<()> {
        if ids.is_empty() { return Ok(()); }
        let (respond_to, response) = oneshot::channel();
        self.notifications.persistence_tx.send(PersistenceMsg::SuppressTerminalCompletions {
            ids: ids.clone(), reason: crate::session::completion_delivery::CompletionSuppression::GoalNotificationPolicy, respond_to,
        }).map_err(|_| std::io::Error::other("completion suppression owner closed"))?;
        response.await.map_err(|_| std::io::Error::other("completion suppression acknowledgement lost"))??;
        self.acknowledge_terminal_receipts(&ids).await;
        Ok(())
    }

    pub(crate) async fn acknowledge_terminal_receipts(&self, ids: &[CompletionIdentity]) {
        for id in ids {
            match id {
                CompletionIdentity::Subagent(_) => {}
                CompletionIdentity::V2 { terminal: TerminalCompletionId::Task { task_id, .. }, .. } => {
                    self.mark_completions_reported(&[task_id.as_str()]).await;
                }
                CompletionIdentity::V2 { terminal: TerminalCompletionId::Workflow { run_id, revision }, .. } => {
                    self.workflow_tracker().await.lock().mark_revision_reported(run_id, *revision);
                }
            }
        }
    }

    pub(crate) async fn project_terminal_input(
        &self,
        input: &InputOrigin,
        prompt_id: &str,
        text: String,
    ) -> Result<Option<bool>, CompletionProjectionError> {
        let mut ids = input.completions().to_vec();
        let mut durable_text = Vec::new();
        if ids.is_empty() && matches!(input.as_prompt_origin(), PromptOrigin::TaskCompleted { .. } | PromptOrigin::WorkflowCompleted { .. }) {
            let state = self.persisted_completion_state().await.map_err(xai_chat_state::StrictAppendError::NotCommitted)?;
            for id in state.obligations.iter().map(|obligation| obligation.id()).chain(state.receipts.iter().map(|receipt| receipt.completion.clone())) {
                let matches = match (&id, input.as_prompt_origin()) {
                    (CompletionIdentity::V2 { terminal: TerminalCompletionId::Task { owner_session_id, task_id, .. }, .. }, PromptOrigin::TaskCompleted { task_id: source }) => owner_session_id == &self.session_id_string() && task_id == source,
                    (CompletionIdentity::V2 { terminal: TerminalCompletionId::Workflow { run_id, revision }, .. }, PromptOrigin::WorkflowCompleted { completion_id }) => completion_id == &format!("{run_id}-{revision}"),
                    _ => false,
                };
                if matches && !ids.contains(&id) {
                    if let Some(payload) = state.obligations.iter().find_map(|obligation| {
                        if obligation.id() != id { return None; }
                        match obligation {
                            CompletionObligation::Pending { payload, .. } | CompletionObligation::Projected { payload, .. } => Some(payload),
                            CompletionObligation::Preparing { .. } | CompletionObligation::Suppressed { .. } => None,
                        }
                    }) {
                        durable_text.push(payload.model_text(&id));
                    }
                    ids.push(id);
                }
            }
        }
        if ids.is_empty() && matches!(input.notification_batch(), super::notification_drain::NotificationBatchPayload::TerminalOnly) { return Ok(None); }
        let text = if durable_text.is_empty() { text } else { durable_text.join("\n\n") };
        self.project_notification_batch(ids, prompt_id, text, input.notification_batch()).await.map(Some)
    }

    pub(crate) async fn project_terminal_batch(
        &self,
        ids: Vec<CompletionIdentity>,
        prompt_id: &str,
        text: String,
    ) -> Result<bool, CompletionProjectionError> {
        self.project_notification_batch(ids, prompt_id, text, &super::notification_drain::NotificationBatchPayload::TerminalOnly).await
    }

    pub(in crate::session::acp_session_impl) async fn project_notification_batch(
        &self,
        ids: Vec<CompletionIdentity>,
        prompt_id: &str,
        text: String,
        payload: &super::notification_drain::NotificationBatchPayload,
    ) -> Result<bool, CompletionProjectionError> {
        let mut changed = false;
        // A concurrent suppression can remove each unprojected identity only once.
        for _ in 0..=ids.len() {
            let state = self.persisted_completion_state().await.map_err(xai_chat_state::StrictAppendError::NotCommitted)?;
            let mut fresh = Vec::new();
            let mut recovery = Vec::new();
            let mut settled = Vec::new();
            for id in ids.iter().cloned() {
                if state.obligations.iter().any(|obligation| matches!(obligation, CompletionObligation::Suppressed { id: suppressed, .. } if suppressed == &id)) { continue; }
                if fresh.contains(&id) || recovery.contains(&id) || settled.contains(&id) { continue; }
                if state.receipts.iter().any(|receipt| receipt.completion == id) {
                    if state.obligations.iter().any(|obligation| obligation.id() == id) { recovery.push(id); }
                    else { settled.push(id); }
                } else { fresh.push(id); }
            }
            self.acknowledge_terminal_receipts(&settled).await;
            for id in recovery {
                changed |= self.commit_terminal_batch(vec![id], prompt_id, text.clone()).await?;
            }
            if fresh.is_empty() {
                if let Some(text) = self.notification_batch_text(payload, &[]) && !text.is_empty() {
                    changed |= self.commit_notification_rows(Vec::new(), prompt_id, text, payload).await?;
                }
            } else {
                let terminal_text: Vec<_> = fresh.iter().map(|id| {
                    let source = state.obligations.iter().find_map(|obligation| {
                        if obligation.id() != *id { return None; }
                        match obligation {
                            CompletionObligation::Pending { payload, .. } | CompletionObligation::Projected { payload, .. } => Some(payload.model_text(id)),
                            CompletionObligation::Preparing { .. } | CompletionObligation::Suppressed { .. } => None,
                        }
                    }).or_else(|| match payload {
                        super::notification_drain::NotificationBatchPayload::TerminalOnly => None,
                        super::notification_drain::NotificationBatchPayload::Rows { rows, .. } => rows.iter().find_map(|row| match row {
                            super::notification_drain::NotificationBatchRow::Terminal { id: row_id, text } if row_id == id => Some(text.clone()),
                            super::notification_drain::NotificationBatchRow::Terminal { .. } | super::notification_drain::NotificationBatchRow::MonitorEvent { .. } => None,
                        }),
                    }).unwrap_or_else(|| text.clone());
                    (id.clone(), source)
                }).collect();
                let text = self.notification_batch_text(payload, &terminal_text).unwrap_or_else(|| terminal_text.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>().join("\n\n"));
                match self.commit_notification_rows(fresh, prompt_id, text, payload).await {
                    Ok(projected) => changed |= projected,
                    Err(xai_chat_state::StrictAppendError::NotCommitted(error)) if matches!(error.kind(), std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::AlreadyExists) => continue,
                    Err(error) => return Err(error),
                }
            }
            return Ok(changed);
        }
        Err(xai_chat_state::StrictAppendError::NotCommitted(std::io::Error::other("completion disposition did not stabilize")))
    }

    async fn commit_notification_rows(
        &self, ids: Vec<CompletionIdentity>, prompt_id: &str, text: String,
        payload: &super::notification_drain::NotificationBatchPayload,
    ) -> Result<bool, CompletionProjectionError> {
        let super::notification_drain::NotificationBatchPayload::Rows { id, .. } = payload else {
            return self.commit_terminal_batch(ids, prompt_id, text).await;
        };
        let admission = self.current_turn_admission(prompt_id).await.map_err(xai_chat_state::StrictAppendError::NotCommitted)?;
        let projection = xai_chat_state::input_projection::NotificationProjection {
            item: ConversationItem::notification_drain(text),
            binding: xai_chat_state::input_projection::NotificationBatchBinding {
                id: id.clone(), prompt_id: prompt_id.to_owned(), parent_task: admission.parent_task.clone(),
            },
            receipts: ids.iter().cloned().map(|completion| xai_chat_state::CompletionReceipt {
                completion, prompt_id: prompt_id.to_owned(), parent_task: admission.parent_task.clone(),
            }).collect(),
        };
        let acknowledgement = self.chat_state_handle.append_notification_and_ack(projection, admission).await?;
        match acknowledgement {
            xai_chat_state::CompletionProjectionAck::Appended | xai_chat_state::CompletionProjectionAck::Continue { .. } => {
                self.acknowledge_terminal_receipts(&ids).await;
                Ok(true)
            }
            xai_chat_state::CompletionProjectionAck::AlreadyPresent { .. } => {
                self.acknowledge_terminal_receipts(&ids).await;
                Ok(false)
            }
            xai_chat_state::CompletionProjectionAck::TurnClosed => Err(xai_chat_state::StrictAppendError::NotCommitted(std::io::Error::new(
                std::io::ErrorKind::Interrupted, "caller closed before notification batch admission",
            ))),
        }
    }

    async fn commit_terminal_batch(
        &self, ids: Vec<CompletionIdentity>, prompt_id: &str, text: String,
    ) -> Result<bool, CompletionProjectionError> {
        let mut ids = ids.into_iter();
        let Some(first) = ids.next() else { return Ok(false); };
        let mut projection = CompletionProjection::new(first, prompt_id.to_owned(), text.clone());
        projection.item = ConversationItem::notification_drain(text);
        projection.additional_receipts = ids.map(|completion| xai_chat_state::CompletionReceipt {
            completion, prompt_id: prompt_id.to_owned(), parent_task: xai_chat_state::ParentTask::Unknown,
        }).collect();
        self.commit_subagent_projection(projection, prompt_id).await
    }

    pub(crate) async fn resume_terminal_completion(
        self: std::sync::Arc<Self>,
        completion_tx: mpsc::UnboundedSender<super::super::turn_task::TurnCompletionMsg>,
        id: CompletionIdentity,
        text: String,
        policy: CompletionEventPolicy,
    ) {
        let offer = CompletionOffer::pending(policy);
        if offer == CompletionOffer::None { return; }
        let projected = match self.persisted_completion_state().await {
            Ok(state) => {
                if state.obligations.iter().any(|obligation| matches!(obligation, CompletionObligation::Suppressed { id: suppressed, .. } if suppressed == &id)) { return; }
                state.receipts.iter().any(|receipt| receipt.completion == id)
            }
            Err(error) => { tracing::error!(%error, "terminal recovery retained until disposition can be read"); return; }
        };
        if !projected && let CompletionIdentity::V2 { terminal: TerminalCompletionId::Task { task_id, task_kind, .. }, .. } = &id {
            let source = match task_kind {
                xai_chat_state::completion::CompletionTaskKind::Bash => NotificationSource::BashTaskCompleted { task_id: task_id.clone() },
                xai_chat_state::completion::CompletionTaskKind::Monitor => NotificationSource::MonitorCompleted { task_id: task_id.clone() },
            };
            if self.retain_or_suppress_task_wake(TaskWakeFallback {
                prompt_id: format!("notifications-recovered-{}", uuid::Uuid::now_v7()),
                prompt_blocks: vec![acp::ContentBlock::from(text.clone())], source,
            }).await.is_none() { return; }
        }
        if self.state.lock().await.pending_inputs.iter().any(|input| input.input_origin.completions().contains(&id)) { return; }
        let origin = InputOrigin::new(PromptOrigin::NotificationDrain).with_completions(vec![id]).with_completion_offer(offer);
        let (respond_to, _) = oneshot::channel();
        let _ = self.queue_input(QueueInputRequest::from_input_origin(
            vec![acp::ContentBlock::Text(acp::TextContent::new(text))],
            format!("notifications-recovered-{}", uuid::Uuid::now_v7()), origin, PromptMode::Agent, respond_to,
        )).await;
        SessionActor::maybe_start_running_task(self, completion_tx).await;
    }

    pub(crate) async fn drain_terminal_wakes_at_safe_point(&self) -> bool {
        let mut changed = false;
        loop {
            let candidate = {
                let state = self.state.lock().await;
                let Some(running) = state.running_prompt_id() else { return changed; };
                state.pending_inputs.iter().find(|input| input.prompt_id != running && !input.input_origin.completions().is_empty())
                    .map(|input| (running.to_owned(), input.prompt_id.clone(), input.input_origin.clone(), Self::queue_text_from_blocks(&input.prompt_blocks)))
            };
            let Some((running, queued, origin, text)) = candidate else { return changed; };
            match self.project_terminal_input(&origin, &running, text).await {
                Ok(Some(projected)) => changed |= projected,
                Ok(None) => return changed,
                Err(error) => { tracing::error!(%error, "terminal wake retained after projection failure"); return changed; }
            }
            let mut state = self.state.lock().await;
            if state.running_prompt_id() != Some(running.as_str()) { return changed; }
            let removed = state.sweep_pending_inputs(|input| input.prompt_id == queued);
            drop(state);
            for input in removed { let _ = input.respond_to.send(ok_end_turn(0, None)); }
        }
    }
}
