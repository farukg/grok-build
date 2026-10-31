//! Registered producer identities captured at parent submission revocation.

#[cfg(test)]
#[path = "parent_revocation_tests.rs"]
mod tests;

use super::{ChildCancellation, ChildRunner, SubagentCoordinator};
use super::super::coordinator_state::TerminalOutput;
use super::super::types::ParentSessionRevocation;
use xai_message_delivery_core::SubagentCompletionId;

impl<R: ChildRunner> SubagentCoordinator<R> {
    pub(super) fn revoke_parent_session(
        &mut self,
        parent: &str,
        intent: ChildCancellation,
    ) -> ParentSessionRevocation {
        let mut completions = self.registered_parent_completions(parent);
        self.cancel_parent_session(Some(parent), intent);
        completions.extend(self.registered_parent_completions(parent));
        completions.sort_unstable_by(|a, b| {
            a.subagent_id.cmp(&b.subagent_id)
                .then_with(|| a.attempt_id.as_str().cmp(b.attempt_id.as_str()))
        });
        completions.dedup();
        ParentSessionRevocation { parent_session_id: parent.to_owned(), completions }
    }

    fn registered_parent_completions(&self, parent: &str) -> Vec<SubagentCompletionId> {
        let mut completions = Vec::new();
        for child in self.pending.values() {
            if child.request.parent_session_id == parent {
                completions.push(SubagentCompletionId {
                    subagent_id: child.request.id.clone(),
                    attempt_id: child.attempt_id.clone(),
                });
            }
        }
        for child in self.active.values() {
            if child.request.parent_session_id == parent {
                completions.push(SubagentCompletionId {
                    subagent_id: child.request.id.clone(),
                    attempt_id: child.attempt_id.clone(),
                });
            }
        }
        for child in self.completed.values() {
            if child.request.parent_session_id == parent {
                completions.push(SubagentCompletionId {
                    subagent_id: child.request.id.clone(),
                    attempt_id: child.attempt_id.clone(),
                });
            }
        }
        let displaced = self.queued.iter().filter_map(|queued| queued.wake.as_ref())
            .chain(self.pending.values().filter_map(|child| match child.wake_of.as_ref() {
                Some(super::super::coordinator_state::PreStartWake::Completed(previous)) => Some(previous),
                Some(super::super::coordinator_state::PreStartWake::Published) | None => None,
            }))
            .chain(self.active.values().filter_map(|child| match child.deferred_wake.as_ref() {
                Some(super::super::coordinator_state::PreStartWake::Completed(previous)) => Some(previous),
                Some(super::super::coordinator_state::PreStartWake::Published) | None => None,
            }));
        for previous in displaced {
            let child = &previous.completed;
            if child.request.parent_session_id == parent {
                completions.push(SubagentCompletionId {
                    subagent_id: child.request.id.clone(),
                    attempt_id: child.attempt_id.clone(),
                });
            }
        }
        for (id, output) in &self.terminal_outputs {
            match output {
                TerminalOutput::Committing { completion, .. } => {
                    if completion.request.parent_session_id == parent {
                        completions.push(SubagentCompletionId {
                            subagent_id: completion.request.id.clone(),
                            attempt_id: completion.attempt_id.clone(),
                        });
                    }
                }
                TerminalOutput::Publishing { attempt_id, parent_session_id, .. } => {
                    if parent_session_id == parent {
                        completions.push(SubagentCompletionId {
                            subagent_id: id.clone(),
                            attempt_id: attempt_id.clone(),
                        });
                    }
                }
                // Finishing retains its registered active record.
                TerminalOutput::Finishing(_) => {}
            }
        }
        for completion in &self.pending_completions {
            if completion.parent_session_id == parent {
                completions.push(SubagentCompletionId {
                    subagent_id: completion.summary.subagent_id().to_owned(),
                    attempt_id: completion.summary.attempt_id.clone(),
                });
            }
        }
        completions
    }
}
