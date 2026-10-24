use super::*;
use crate::session::completion_delivery::{CompletionWake, CompletionWakeKind};
use xai_chat_state::{
    CompletionProjection, CompletionProjectionAck, CompletionProjectionError, SubagentCompletionId,
};
use xai_grok_tools::implementations::grok_build::task::types::{
    SubagentCompletionsRequest, SubagentEvent,
};

impl SessionActor {
    pub(super) async fn persisted_completion_state(
        &self,
    ) -> std::io::Result<crate::session::completion_delivery::CompletionDeliveryState> {
        let (respond_to, receive) = oneshot::channel();
        self.notifications
            .persistence_tx
            .send(PersistenceMsg::GetCompletionState { respond_to })
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "completion state owner closed",
                )
            })?;
        receive.await.map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "completion state observation lost",
            )
        })?
    }

    pub(super) async fn input_turn_binding(&self, prompt_id: &str) -> InputTurnBinding {
        let state = self.state.lock().await;
        match state
            .running_task
            .as_ref()
            .filter(|task| task.prompt_id == prompt_id)
        {
            Some(task) => InputTurnBinding {
                parent_task: xai_chat_state::ParentTask::Instance {
                    id: task.identity.as_ref().clone(),
                },
                receiver_origin: crate::session::delivery_receiver::ReceiverTurnOrigin::from_prompt(
                    task.input_origin.as_prompt_origin(),
                ),
            },
            None => InputTurnBinding {
                parent_task: xai_chat_state::ParentTask::Unknown,
                receiver_origin: state
                    .pending_inputs
                    .front()
                    .filter(|item| item.prompt_id == prompt_id)
                    .map(|item| {
                        crate::session::delivery_receiver::ReceiverTurnOrigin::from_prompt(
                            item.input_origin.as_prompt_origin(),
                        )
                    })
                    .unwrap_or_default(),
            },
        }
    }

    pub(super) async fn current_turn_admission(
        &self,
        prompt_id: &str,
    ) -> std::io::Result<xai_chat_state::InputAdmission> {
        let state = self.state.lock().await;
        let task = state
            .running_task
            .as_ref()
            .filter(|task| task.prompt_id == prompt_id)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "input has no live owning caller task",
                )
            })?;
        Ok(xai_chat_state::InputAdmission {
            parent_task: xai_chat_state::ParentTask::Instance {
                id: task.identity.as_ref().clone(),
            },
            cancellation: task.admission.clone(),
        })
    }

    pub(super) fn set_completion_wake_policy(
        &self,
        policy: crate::session::completion_delivery::CompletionWakePolicy,
    ) {
        if let Some(gate) = &self.tool_context.task_wake_suppressed {
            gate.set(
                policy == crate::session::completion_delivery::CompletionWakePolicy::HumanStopped,
            );
        }
        let _ = self
            .notifications
            .persistence_tx
            .send(PersistenceMsg::CompletionWakePolicy(policy));
    }

    pub(super) async fn project_subagent_completion(
        &self,
        completion: &CompletionWake,
        prompt_id: &str,
        text: String,
    ) -> Result<bool, CompletionProjectionError> {
        let projection =
            CompletionProjection::new(completion.id.clone(), prompt_id.to_owned(), text);
        self.commit_subagent_projection(projection, prompt_id).await
    }

    pub(super) async fn commit_subagent_projection(
        &self,
        projection: CompletionProjection,
        prompt_id: &str,
    ) -> Result<bool, CompletionProjectionError> {
        let completions: Vec<_> = projection
            .receipts()
            .map(|receipt| receipt.completion.clone())
            .collect();
        let admission = self
            .current_turn_admission(prompt_id)
            .await
            .map_err(xai_chat_state::StrictAppendError::NotCommitted)?;
        match self
            .chat_state_handle
            .append_completion_and_ack(projection, admission)
            .await?
        {
            CompletionProjectionAck::Appended | CompletionProjectionAck::Continue { .. } => {
                for completion in &completions {
                    self.acknowledge_subagent_completion(completion);
                }
                Ok(true)
            }
            CompletionProjectionAck::AlreadyPresent { .. } => {
                for completion in &completions {
                    self.acknowledge_subagent_completion(completion);
                }
                Ok(false)
            }
            CompletionProjectionAck::TurnClosed => Ok(false),
        }
    }

    pub(super) async fn commit_observed_subagent_tool_result(
        &self,
        item: ConversationItem,
        observations: Vec<
            xai_grok_tools::types::completion_observation::ObservedSubagentCompletion,
        >,
    ) -> Result<(), acp::Error> {
        let mut observations = observations.into_iter();
        let Some(first) = observations.next() else {
            self.chat_state_handle.push_tool_result(item);
            return Ok(());
        };
        let prompt_id = self
            .current_prompt_id
            .lock()
            .ok()
            .and_then(|id| id.clone())
            .ok_or_else(|| {
                acp::Error::internal_error().data("observed child result has no owning parent turn")
            })?;
        let mut projection = CompletionProjection::new(
            SubagentCompletionId {
                subagent_id: first.subagent_id,
                attempt_id: first.attempt_id,
            },
            prompt_id.clone(),
            String::new(),
        );
        projection.item = item;
        projection.additional_receipts = observations
            .map(|observed| xai_chat_state::CompletionReceipt {
                completion: SubagentCompletionId {
                    subagent_id: observed.subagent_id,
                    attempt_id: observed.attempt_id,
                },
                prompt_id: prompt_id.clone(),
                parent_task: xai_chat_state::ParentTask::Unknown,
            })
            .collect();
        let completions: Vec<_> = projection
            .receipts()
            .map(|r| r.completion.clone())
            .collect();
        let admission = self
            .current_turn_admission(&prompt_id)
            .await
            .map_err(|error| acp::Error::internal_error().data(error.to_string()))?;
        let acknowledgement = self
            .chat_state_handle
            .append_completion_and_ack(projection, admission)
            .await
            .map_err(|error| {
                acp::Error::internal_error().data(format!("child tool-result persistence: {error}"))
            })?;
        match acknowledgement {
            CompletionProjectionAck::TurnClosed => {
                return Err(acp::Error::internal_error()
                    .data("parent turn ended before child tool-result admission"));
            }
            CompletionProjectionAck::Appended
            | CompletionProjectionAck::AlreadyPresent { .. }
            | CompletionProjectionAck::Continue { .. } => {}
        }
        for completion in completions {
            self.acknowledge_subagent_completion(&completion);
        }
        Ok(())
    }

    fn acknowledge_subagent_completion(&self, completion: &SubagentCompletionId) {
        if let Some(sender) = &self.tool_context.subagent_event_tx {
            let _ = sender.send(SubagentEvent::AcknowledgeCompletions {
                parent_session_id: self.session_id_string(),
                completions: vec![completion.clone()],
            });
        }
    }

    pub(super) async fn drain_subagent_wakes_at_safe_point(&self) -> bool {
        if self
            .tool_context
            .task_wake_suppressed
            .as_ref()
            .is_some_and(|gate| gate.get())
        {
            return false;
        }
        let mut changed = false;
        loop {
            let mut state = self.state.lock().await;
            let Some(running_id) = state.running_prompt_id().map(str::to_owned) else {
                return changed;
            };
            let candidate = state.pending_inputs.iter().find_map(|item| {
                match item.input_origin.as_prompt_origin() {
                    crate::session::PromptOrigin::SubagentCompleted { completion }
                        if item.prompt_id != running_id =>
                    {
                        Some((
                            item.prompt_id.clone(),
                            completion.clone(),
                            Self::queue_text_from_blocks(&item.prompt_blocks),
                        ))
                    }
                    _ => None,
                }
            });
            let Some((queued_id, completion, text)) = candidate else {
                drop(state);
                return self.drain_buffered_subagent_completions(&running_id).await || changed;
            };
            drop(state);
            match self
                .project_subagent_completion(&completion, &running_id, text)
                .await
            {
                Ok(projected) => changed |= projected,
                Err(error) => {
                    tracing::error!(%error, attempt_id = %completion.id.attempt_id, "subagent completion remains pending after persistence failure");
                    return changed;
                }
            }
            let mut state = self.state.lock().await;
            if state.running_prompt_id() != Some(running_id.as_str()) {
                return changed;
            }
            let removed = state.sweep_pending_inputs(|item| item.prompt_id == queued_id);
            drop(state);
            for item in removed {
                let _ = item.respond_to.send(ok_end_turn(0, None));
            }
        }
    }

    pub(super) async fn drain_buffered_subagent_completions(&self, prompt_id: &str) -> bool {
        if self
            .tool_context
            .task_wake_suppressed
            .as_ref()
            .is_some_and(|gate| gate.get())
        {
            return false;
        }
        let Some(sender) = &self.tool_context.subagent_event_tx else {
            return false;
        };
        let (respond_to, receive) = oneshot::channel();
        if sender
            .send(SubagentEvent::PeekCompletions(SubagentCompletionsRequest {
                parent_session_id: Some(self.session_id_string()),
                suppress_ids: Vec::new(),
                respond_to,
            }))
            .is_err()
        {
            return false;
        }
        let Ok(completions) = receive.await else {
            return false;
        };
        if completions.is_empty() {
            return false;
        }
        let persisted = match self.persisted_completion_state().await {
            Ok(persisted) => persisted,
            Err(error) => {
                tracing::error!(%error, "buffered completion remains pending after state observation failure");
                return false;
            }
        };
        let summary_path =
            crate::session::persistence::session_dir(&self.session_info).join("summary.json");
        let bridge = self.agent.borrow().tool_bridge().clone();
        let mut changed = false;
        for summary in completions {
            let attempt_id = &summary.attempt_id;
            if !xai_grok_tools::implementations::grok_build::task::types::completion_state_permits_wake(&summary.snapshot.status.state()) {
                continue;
            }
            let completion = CompletionWake {
                id: SubagentCompletionId {
                    subagent_id: summary.subagent_id().to_owned(),
                    attempt_id: attempt_id.clone(),
                },
                kind: CompletionWakeKind::Fresh,
            };
            let reminder = xai_grok_tools::reminders::task_completion::format_between_turn_completion_reminder(
                &[summary], &bridge,
            ).await;
            let mut text = reminder_text(&reminder, self.reminder_wrapper_tag());
            if let Some(reference) = persisted.result_reference(&completion.id, &summary_path) {
                text.push_str("\n\n");
                text.push_str(
                    &crate::session::completion_delivery::completion_reference_text(
                        &completion.id,
                        &reference,
                    ),
                );
            }
            match self
                .project_subagent_completion(&completion, prompt_id, text)
                .await
            {
                Ok(projected) => changed |= projected,
                Err(error) => {
                    tracing::error!(%error, attempt_id = %completion.id.attempt_id, "buffered child result remains unacknowledged")
                }
            }
        }
        changed
    }
}
