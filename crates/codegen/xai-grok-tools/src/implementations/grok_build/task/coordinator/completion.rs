//! Between-turn completion buffering: which session is told a child finished.

use super::super::coordinator_state::{
    BufferedCompletion, CompletionBufferPublication, CompletionRecipient, CompletionReporting,
    TerminalOutput, cap_completion_output, completion_summary,
};
use super::super::types::{SubagentRequest, SubagentResult, SubagentSnapshot};
use super::{ChildRunner, SubagentCoordinator};

/// Sessions unloaded without a `TeardownSession` must not grow the buffer unboundedly.
const MAX_PENDING_COMPLETIONS: usize = 256;

impl<R: ChildRunner> SubagentCoordinator<R> {
    pub(super) fn completion_recipient(&self, request: &SubagentRequest) -> CompletionRecipient {
        let target = if request.surface_completion {
            // Only direct workflow children skip: nested ones inherit workflow
            // ownership at reparent yet still owe their spawner a reminder.
            if request.owner.is_workflow() {
                return CompletionRecipient::Silent;
            }
            Some(request.parent_session_id.as_str())
        } else {
            self.graph
                .surface_target(&request.id)
                .filter(|spawner| self.is_spawner_live(spawner))
        };
        match target {
            Some(session) => CompletionRecipient::Live(session.to_owned()),
            None => CompletionRecipient::Silent,
        }
    }

    pub(super) fn prepare_buffered_completion(
        &self,
        request: &SubagentRequest,
        result: &SubagentResult,
        snapshot: &SubagentSnapshot,
        attempt_id: &xai_message_delivery_core::AttemptId,
        recipient: &CompletionRecipient,
    ) -> CompletionBufferPublication {
        if !self.config.buffer_completions {
            return CompletionBufferPublication::Silent;
        }
        let parent_session_id = match recipient {
            CompletionRecipient::Live(session) => session,
            CompletionRecipient::Retired(_) | CompletionRecipient::Silent => {
                return CompletionBufferPublication::Silent;
            }
        };
        if !request.surface_completion && !self.is_spawner_live(parent_session_id) {
            return CompletionBufferPublication::Silent;
        }
        let mut summary = completion_summary(attempt_id, request, result, snapshot);
        if let Some(cap) = self.config.buffered_completion_output_cap {
            summary.output = cap_completion_output(&summary.output, cap);
        }
        CompletionBufferPublication::Pending(BufferedCompletion {
            parent_session_id: parent_session_id.to_owned(),
            summary,
        })
    }

    pub(super) fn publish_buffered_completion(&mut self, publication: CompletionBufferPublication) {
        match publication {
            CompletionBufferPublication::Silent => return,
            CompletionBufferPublication::Pending(completion) => {
                self.pending_completions.push(completion)
            }
        }
        if self.pending_completions.len() > MAX_PENDING_COMPLETIONS {
            let excess = self.pending_completions.len() - MAX_PENDING_COMPLETIONS;
            self.pending_completions.drain(..excess);
        }
    }

    /// A finished spawner has no reader left.
    pub(super) fn purge_completions_for_spawner(&mut self, finished_session: &str) {
        self.pending_completions
            .retain(|completion| completion.parent_session_id != finished_session);
        for output in self.terminal_outputs.values_mut() {
            match output {
                TerminalOutput::Finishing(_) => {}
                TerminalOutput::Committing { completion, recipient } => {
                    if completion.request.parent_session_id == finished_session
                        || recipient.session_id() == Some(finished_session)
                    {
                        completion.disposition.reporting = CompletionReporting::Silent;
                        let session = recipient.session_id().unwrap_or(finished_session).to_owned();
                        *recipient = CompletionRecipient::Retired(session);
                    }
                }
                TerminalOutput::Publishing { recipient, buffered, .. } => {
                    recipient.retire_parent(finished_session);
                    buffered.retire_parent(finished_session);
                }
            }
        }
    }

    /// Cancelled counts as gone: nothing will read the buffer.
    fn is_spawner_live(&self, spawner_session_id: &str) -> bool {
        self.active_child_for_session(spawner_session_id)
            .is_some_and(|child| !child.cancellation.is_cancelled())
    }
}
