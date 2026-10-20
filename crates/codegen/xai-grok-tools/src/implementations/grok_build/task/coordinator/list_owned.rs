//! Every subagent a session can reach, in every lifecycle phase.

use tokio::sync::oneshot;

use super::super::coordinator_state::{completed_inspection, pending_inspection, queued_inspection};
use super::super::types::{SubagentInspection, SubagentRequest};
use super::{ChildRunner, SubagentCoordinator};

impl<R: ChildRunner> SubagentCoordinator<R> {
    /// Finished children are listed from memory without their persisted output; running ones report live progress.
    pub(super) fn handle_list_owned(
        &mut self,
        parent_session_id: Option<&str>,
        respond_to: oneshot::Sender<Vec<SubagentInspection>>,
    ) {
        let listed = |request: &SubagentRequest| {
            !request.owner.is_workflow()
                && self.is_reachable_from_session(&request.id, parent_session_id)
        };
        let known: Vec<Option<SubagentInspection>> = self
            .completed
            .values()
            .filter(|child| listed(&child.request))
            .map(|child| completed_inspection(child, None))
            .chain(
                self.pending
                    .values()
                    .filter(|child| listed(&child.request))
                    .map(pending_inspection),
            )
            .chain(
                self.queued
                    .iter()
                    .filter(|queued| listed(&queued.request))
                    .map(|queued| queued_inspection(&queued.request, queued.queued_at.into_std())),
            )
            .map(Some)
            .collect();
        let active: Vec<String> = self
            .active
            .values()
            .filter(|child| listed(&child.request))
            .map(|child| child.request.id.clone())
            .collect();
        self.respond_with_active_progress(known, active, respond_to);
    }
}

#[cfg(test)]
#[path = "list_owned_tests.rs"]
mod tests;
