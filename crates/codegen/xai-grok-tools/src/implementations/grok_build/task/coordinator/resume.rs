//! `resume_from` reference resolution against live, finished, and on-disk subagents.

use super::super::resume::{SubagentReferenceMatch, match_subagent_reference};
use super::super::types::{SubagentResumeError, SubagentResumeTarget};
use super::{ChildRunner, SubagentCoordinator};

impl<R: ChildRunner> SubagentCoordinator<R> {
    /// Memory first (bounded by the live maps and `MAX_COMPLETED_ENTRIES`); only a miss reads the
    /// parent's `subagents/` directory names.
    pub(super) fn resolve_resume(
        &self,
        reference: &str,
        parent_session_id: &str,
    ) -> Result<SubagentResumeTarget, SubagentResumeError> {
        let reference = reference.trim();
        if !xai_tool_types::is_not_sentinel(reference) {
            return Err(SubagentResumeError::NotFound {
                reference: reference.to_owned(),
            });
        }
        let owned = |id: &str| self.graph.is_reachable_from(id, parent_session_id);
        match match_subagent_reference(reference, self.resume_keys().filter(|&(_, id)| owned(id)))
        {
            SubagentReferenceMatch::Unique(subagent_id) => {
                return Ok(self.classify_live_or_finished(subagent_id));
            }
            SubagentReferenceMatch::Ambiguous(candidates) => {
                return Err(SubagentResumeError::Ambiguous {
                    reference: reference.to_owned(),
                    candidates,
                });
            }
            SubagentReferenceMatch::NoMatch => {}
        }
        // A nested spawner's children are persisted under the root session.
        let disk_parent = self
            .active_child_for_session(parent_session_id)
            .map_or(parent_session_id, |spawner| {
                spawner.request.parent_session_id.as_str()
            });
        let on_disk = self
            .runner
            .durable_resume_candidates(reference, disk_parent);
        match match_subagent_reference(reference, on_disk.iter().map(|id| (id.as_str(), id.as_str())))
        {
            SubagentReferenceMatch::Unique(subagent_id) => {
                self.runner.durable_resume_check(&subagent_id, disk_parent)?;
                Ok(SubagentResumeTarget::Finished { subagent_id })
            }
            SubagentReferenceMatch::Ambiguous(candidates) => Err(SubagentResumeError::Ambiguous {
                reference: reference.to_owned(),
                candidates,
            }),
            SubagentReferenceMatch::NoMatch => {
                match match_subagent_reference(reference, self.resume_keys()) {
                    SubagentReferenceMatch::Unique(subagent_id) => {
                        Err(SubagentResumeError::ForeignParent { subagent_id })
                    }
                    SubagentReferenceMatch::Ambiguous(_) | SubagentReferenceMatch::NoMatch => {
                        Err(SubagentResumeError::NotFound {
                            reference: reference.to_owned(),
                        })
                    }
                }
            }
        }
    }

    /// `(key, subagent_id)` for every known subagent; keys are subagent and child session ids.
    fn resume_keys(&self) -> impl Iterator<Item = (&str, &str)> {
        let pending = self
            .pending
            .values()
            .map(|child| (child.request.id.as_str(), child.request.id.as_str()));
        let queued = self
            .queued
            .iter()
            .map(|queued| (queued.request.id.as_str(), queued.request.id.as_str()));
        let active = self.active.values().flat_map(|child| {
            [
                (child.request.id.as_str(), child.request.id.as_str()),
                (child.child_session_id.as_str(), child.request.id.as_str()),
            ]
        });
        let completed = self.completed.values().flat_map(|child| {
            [
                (child.request.id.as_str(), child.request.id.as_str()),
                (child.child_session_id.as_str(), child.request.id.as_str()),
            ]
        });
        pending.chain(queued).chain(active).chain(completed)
    }

    fn classify_live_or_finished(&self, subagent_id: String) -> SubagentResumeTarget {
        let live = self.active.contains_key(&subagent_id)
            || self.pending.contains_key(&subagent_id)
            || self.queued.contains_id(&subagent_id);
        if live {
            SubagentResumeTarget::Running { subagent_id }
        } else {
            SubagentResumeTarget::Finished { subagent_id }
        }
    }
}
