use super::*;
use xai_chat_state::{CompletionProjection, CompletionReceipt, StrictAppendError};
use xai_grok_tools::implementations::grok_build::task::types::{SubagentCompletionsRequest, SubagentEvent, completion_state_permits_wake};

impl SessionActor {
    pub(super) async fn project_subagent_wake_turn(
        &self,
        completion: &crate::session::CompletionWake,
        prompt_id: &str,
        fallback: String,
    ) -> Result<bool, xai_chat_state::CompletionProjectionError> {
        if self.tool_context.task_wake_suppressed.as_ref().is_some_and(|gate| gate.get()) { return Ok(false); }
        let (respond_to, receive) = oneshot::channel();
        self.notifications.persistence_tx.send(PersistenceMsg::GetCompletionReceipts { respond_to })
            .map_err(|_| StrictAppendError::NotCommitted(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "completion receipt owner closed")))?;
        let receipts = receive.await.map_err(|_| StrictAppendError::Indeterminate(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "completion receipt observation lost")))?
            .map_err(StrictAppendError::NotCommitted)?;
        if receipts.iter().any(|receipt| receipt.completion == completion.id) {
            return self.project_subagent_completion(completion, prompt_id, fallback).await;
        }
        let Some(sender) = &self.tool_context.subagent_event_tx else {
            return self.project_subagent_completion(completion, prompt_id, fallback).await;
        };
        let (respond_to, receive) = oneshot::channel();
        if sender.send(SubagentEvent::PeekCompletions(SubagentCompletionsRequest {
            parent_session_id: Some(self.session_id_string()),
            suppress_ids: receipts.iter().map(|receipt| receipt.completion.clone()).collect(), respond_to,
        })).is_err() {
            return self.project_subagent_completion(completion, prompt_id, fallback).await;
        }
        let summaries = receive.await.map_err(|_| StrictAppendError::NotCommitted(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "completion observation lost")))?;
        let summaries: Vec<_> = summaries.into_iter().filter(|summary| completion_state_permits_wake(&summary.snapshot.status.state())).collect();
        let bridge = self.agent.borrow().tool_bridge().clone();
        let text = if summaries.is_empty() { fallback } else {
            let contains_primary = summaries.iter().any(|summary| summary.subagent_id() == completion.id.subagent_id && summary.attempt_id == completion.id.attempt_id);
            let digest = xai_grok_tools::reminders::task_completion::format_between_turn_completion_reminder(&summaries, &bridge).await;
            let digest = reminder_text(&digest, self.reminder_wrapper_tag());
            if contains_primary { digest } else { format!("{fallback}\n\n{digest}") }
        };
        let mut projection = CompletionProjection::new(completion.id.clone(), prompt_id.to_owned(), text);
        projection.additional_receipts = summaries.into_iter().filter(|summary| summary.subagent_id() != completion.id.subagent_id || summary.attempt_id != completion.id.attempt_id)
            .map(|summary| CompletionReceipt {
                completion: xai_chat_state::SubagentCompletionId { subagent_id: summary.subagent_id().to_owned(), attempt_id: summary.attempt_id },
                prompt_id: prompt_id.to_owned(), parent_task: xai_chat_state::ParentTask::Unknown,
            }).collect();
        self.commit_subagent_projection(projection, prompt_id).await
    }
}
