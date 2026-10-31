use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::future::JoinAll;

use xai_chat_state::source_version::SourceVersion;

use super::graph::RuntimeInventory;
use super::{ChildRunner, SubagentCoordinator};
use super::super::coordinator_state::{
    ChildControl, RunningSeed, TerminalOutput, completed_inspection, completed_resident_inspection, pending_inspection, queued_inspection,
    resident_inspection, running_inspection, running_seed,
};
use super::super::inventory::{InventoryIssue, InventoryValidationError, ResidentChildTarget};
use super::super::types::{
    ChildObservation, SubagentAccessError, SubagentFinalization, SubagentInspection,
    SubagentSnapshotStatus,
};

pub(crate) struct RuntimeInventoryCapture {
    pub(crate) version: SourceVersion,
    pub(crate) runtime: Vec<RuntimeInventory>,
}

enum CaptureSlot<F> {
    Ready(std::future::Ready<Result<RuntimeInventory, SubagentAccessError>>),
    Active { seed: RunningSeed, target: ResidentChildTarget, progress: Pin<Box<F>> },
    Completed,
}

impl<F: Future<Output = super::super::coordinator_state::SubagentProgress>> Future for CaptureSlot<F> {
    type Output = Result<RuntimeInventory, SubagentAccessError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let result = match this {
            Self::Ready(future) => Pin::new(future).poll(cx),
            Self::Active { seed, target, progress } => progress.as_mut().poll(cx).map(|progress| {
                let inspection = running_inspection(seed.clone(), progress, target.clone());
                inspection.validate().map_err(invalid)?;
                Ok(RuntimeInventory { control_owner: inspection.recorded_owner.clone(), inspection })
            }),
            Self::Completed => Poll::Pending,
        };
        if result.is_ready() {
            *this = Self::Completed;
        }
        result
    }
}

enum CaptureState<F: Future<Output = super::super::coordinator_state::SubagentProgress>> {
    Rejected(std::future::Ready<Result<RuntimeInventoryCapture, SubagentAccessError>>),
    Collecting { version: SourceVersion, slots: JoinAll<CaptureSlot<F>> },
    Completed,
}

pub(crate) struct RuntimeInventoryCaptureFuture<F: Future<Output = super::super::coordinator_state::SubagentProgress>> {
    state: CaptureState<F>,
}

impl<F: Future<Output = super::super::coordinator_state::SubagentProgress>> Future for RuntimeInventoryCaptureFuture<F> {
    type Output = Result<RuntimeInventoryCapture, SubagentAccessError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let result = match &mut this.state {
            CaptureState::Rejected(future) => Pin::new(future).poll(cx),
            CaptureState::Collecting { version, slots } => Pin::new(slots).poll(cx).map(|values| {
                let runtime = values.into_iter().collect::<Result<Vec<_>, _>>()?;
                Ok(RuntimeInventoryCapture { version: *version, runtime })
            }),
            CaptureState::Completed => Poll::Pending,
        };
        if result.is_ready() {
            this.state = CaptureState::Completed;
        }
        result
    }
}

impl<R: ChildRunner> SubagentCoordinator<R> {
    pub(crate) fn capture_runtime_inventory(
        &self,
        parent_session_id: &str,
        child_ids: &[String],
    ) -> RuntimeInventoryCaptureFuture<<R::Control as ChildControl>::ProgressFuture> {
        let state = match self.prepare_runtime_inventory_capture(parent_session_id, child_ids) {
            Ok((version, slots)) => CaptureState::Collecting {
                version, slots: futures::future::join_all(slots),
            },
            Err(error) => CaptureState::Rejected(std::future::ready(Err(error))),
        };
        RuntimeInventoryCaptureFuture { state }
    }

    fn prepare_runtime_inventory_capture(
        &self,
        parent_session_id: &str,
        child_ids: &[String],
    ) -> Result<(SourceVersion, Vec<CaptureSlot<<R::Control as ChildControl>::ProgressFuture>>), SubagentAccessError> {
        let version = self.graph.inventory_source_version(parent_session_id)?;
        let mut requested = HashSet::with_capacity(child_ids.len());
        for id in child_ids {
            super::super::inventory::validate_id(id).map_err(invalid)?;
            if !requested.insert(id.as_str()) {
                return Err(invalid(InventoryValidationError::InvalidIdentity));
            }
        }
        let mut captured = Vec::with_capacity(child_ids.len());
        for id in child_ids {
            if self.graph.root_session(id).is_some_and(|root| root != parent_session_id) {
                return Err(invalid(InventoryValidationError::IdentityMismatch));
            }
            let Some(target) = self.runtime_inventory_target(id)? else { continue; };
            if self.graph.root_session(id) != Some(parent_session_id) {
                return Err(invalid(InventoryValidationError::IdentityMismatch));
            }
            let inspection = match self.terminal_outputs.get(id) {
                Some(TerminalOutput::Committing { completion, .. }) => {
                    let mut snapshot = completion.snapshot.clone();
                    snapshot.status = SubagentSnapshotStatus::Finalizing {
                        publication: SubagentFinalization::Writing,
                    };
                    resident_inspection(
                        snapshot, &completion.request,
                        Some(completion.result.child_session_id.clone()),
                        completion.request.resume_from.clone(), target,
                    )
                }
                Some(TerminalOutput::Publishing { .. }) => {
                    let child = self.completed.get(id).ok_or_else(|| unavailable(id))?;
                    let mut inspection = completed_resident_inspection(child, None, target);
                    set_finalizing(&mut inspection, SubagentFinalization::Publishing)?;
                    inspection
                }
                Some(TerminalOutput::Finishing(output)) => {
                    let mut inspection = if let Some(child) = self.active.get(id) {
                        resident_inspection(
                            super::super::types::SubagentSnapshot {
                                subagent_id: child.request.id.clone(),
                                description: child.request.description.clone(),
                                subagent_type: child.request.subagent_type.clone(),
                                status: SubagentSnapshotStatus::Finalizing {
                                    publication: SubagentFinalization::Settling,
                                },
                                started_at_epoch_ms: super::super::coordinator_state::instant_to_epoch_ms(child.started_at),
                                duration_ms: output.result.duration_ms,
                                persona: child.persona.clone(),
                            },
                            &child.request, Some(child.child_session_id.clone()),
                            child.resumed_from.clone(),
                            target,
                        )
                    } else if let Some(child) = self.pending.get(id) {
                        pending_inspection(child, target)
                    } else {
                        return Err(unavailable(id));
                    };
                    set_finalizing(&mut inspection, SubagentFinalization::Settling)?;
                    inspection
                }
                None => {
                    if let Some(child) = self.active.get(id) {
                        if child.request.id != *id || child.request.parent_session_id != parent_session_id {
                            return Err(invalid(InventoryValidationError::IdentityMismatch));
                        }
                        captured.push(CaptureSlot::Active {
                            seed: running_seed(child), target, progress: Box::pin(child.control.progress()),
                        });
                        continue;
                    } else if let Some(child) = self.pending.get(id) {
                        pending_inspection(child, target)
                    } else if let Some(entry) = self.queued.get(id) {
                        queued_inspection(&entry.request, entry.queued_at.into_std(),
                            target)
                    } else if let Some(child) = self.completed.get(id) {
                        completed_inspection(child, None, target, version).map_err(invalid)?
                    } else {
                        return Err(unavailable(id));
                    }
                }
            };
            self.validate_inventory_capture(parent_session_id, id, version, &inspection)?;
            captured.push(CaptureSlot::Ready(std::future::ready(Ok(RuntimeInventory {
                control_owner: inspection.recorded_owner.clone(), inspection,
            }))));
        }
        Ok((version, captured))
    }

    fn validate_inventory_capture(
        &self,
        parent: &str,
        id: &str,
        version: SourceVersion,
        inspection: &SubagentInspection,
    ) -> Result<(), SubagentAccessError> {
        if inspection.subagent_id != id || inspection.parent_session_id != parent
            || self.graph.root_session(id) != Some(parent)
        {
            return Err(invalid(InventoryValidationError::IdentityMismatch));
        }
        if self.graph.inventory_source_version(parent)? != version {
            return Err(SubagentAccessError::InventoryChanged);
        }
        inspection.validate().map_err(invalid)
    }
}

fn set_finalizing(
    inspection: &mut SubagentInspection,
    publication: SubagentFinalization,
) -> Result<(), SubagentAccessError> {
    match &mut inspection.observation {
        ChildObservation::Resident { status, .. } => {
            *status = SubagentSnapshotStatus::Finalizing { publication };
            Ok(())
        }
        ChildObservation::Terminal { .. } | ChildObservation::IdentityOnly { .. }
        | ChildObservation::Uncustodied { .. } => Err(invalid(InventoryValidationError::InvalidEvidence)),
    }
}

pub(super) fn unavailable(id: &str) -> SubagentAccessError {
    SubagentAccessError::InventoryUnreadable {
        issues: vec![InventoryIssue::RuntimeCaptureUnavailable { subagent_id: id.to_owned() }],
    }
}

fn invalid(reason: InventoryValidationError) -> SubagentAccessError {
    SubagentAccessError::InvalidInventory { reason }
}

#[cfg(test)]
#[path = "inventory_capture_tests.rs"]
mod tests;
