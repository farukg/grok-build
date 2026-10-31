use xai_chat_state::SubagentCompletionId;
use xai_chat_state::source_version::SourceVersion;

use super::*;
use super::super::inventory::{
    AttemptEvidence, ChildEvidenceSource, InventoryValidationError, RecordedSubagentOwner,
    ResidentChildTarget,
};
use super::super::terminal::{
    ChildTerminalEvidence, RecordedSubagentTerminal, RecordedTerminalState, SubagentTerminal,
    SubagentTerminalState,
};
use super::super::types::{ChildObservation, SubagentOwner};

pub(super) fn recorded_owner(owner: &SubagentOwner) -> RecordedSubagentOwner {
    match owner {
        SubagentOwner::Task => RecordedSubagentOwner::Task,
        SubagentOwner::Workflow { run_id } => RecordedSubagentOwner::Workflow {
            run_id: run_id.clone(),
        },
    }
}

pub(in super::super) fn resident_inspection(
    snapshot: SubagentSnapshot,
    request: &SubagentRequest,
    child_session_id: Option<String>,
    resumed_from: Option<String>,
    target: ResidentChildTarget,
) -> SubagentInspection {
    let (started_at_epoch_ms, child_session_id) = match &target.attempt {
        AttemptEvidence::NotStarted => (None, None),
        AttemptEvidence::Known(_) | AttemptEvidence::Legacy => (Some(snapshot.started_at_epoch_ms), child_session_id),
    };
    SubagentInspection {
        subagent_id: snapshot.subagent_id,
        description: Some(snapshot.description),
        subagent_type: Some(snapshot.subagent_type),
        started_at_epoch_ms,
        persona: snapshot.persona,
        parent_session_id: request.parent_session_id.clone(),
        child_session_id,
        fork_parent_prompt_id: request.parent_prompt_id.clone(),
        resumed_from,
        attempt: target.attempt.clone(),
        recorded_owner: Some(recorded_owner(&request.owner)),
        recorded_spawner: request.recorded_spawner.clone(),
        observation: ChildObservation::Resident {
            status: snapshot.status,
            duration_ms: snapshot.duration_ms,
            target,
        },
    }
}

pub(in super::super) fn running_inspection(
    seed: RunningSeed,
    progress: SubagentProgress,
    target: ResidentChildTarget,
) -> SubagentInspection {
    SubagentInspection {
        subagent_id: seed.subagent_id,
        description: Some(seed.description),
        subagent_type: Some(seed.subagent_type),
        started_at_epoch_ms: Some(seed.started_at_epoch_ms),
        persona: seed.persona,
        parent_session_id: seed.parent_session_id,
        child_session_id: Some(seed.child_session_id),
        fork_parent_prompt_id: seed.fork_parent_prompt_id,
        resumed_from: seed.resumed_from,
        attempt: seed.attempt,
        recorded_owner: Some(seed.recorded_owner),
        recorded_spawner: seed.recorded_spawner,
        observation: ChildObservation::Resident {
            status: SubagentSnapshotStatus::Running {
                turn_count: progress.turn_count,
                tool_call_count: progress.tool_call_count,
                tokens_used: progress.tokens_used,
                context_window_tokens: progress.context_window_tokens,
                context_usage_pct: progress.context_usage_pct,
                tools_used: progress.tools_used,
                error_count: progress.error_count,
            },
            duration_ms: seed.duration_ms,
            target,
        },
    }
}

pub(in super::super) fn pending_inspection(
    child: &PendingChild,
    target: ResidentChildTarget,
) -> SubagentInspection {
    resident_inspection(
        pending_snapshot(child), &child.request, None, child.request.resume_from.clone(), target,
    )
}

pub(in super::super) fn queued_inspection(
    request: &SubagentRequest,
    queued_at: std::time::Instant,
    target: ResidentChildTarget,
) -> SubagentInspection {
    resident_inspection(
        queued_snapshot(request, queued_at), request, None, request.resume_from.clone(), target,
    )
}

pub(in super::super) fn completed_resident_inspection(
    child: &CompletedChild,
    persisted_output: Option<&str>,
    target: ResidentChildTarget,
) -> SubagentInspection {
    let mut snapshot = completed_snapshot(child, persisted_output);
    match &child.publication {
        TerminalPublicationState::Publishing => {
            snapshot.status = SubagentSnapshotStatus::Finalizing {
                publication: super::super::types::SubagentFinalization::Publishing,
            };
        }
        TerminalPublicationState::Committing | TerminalPublicationState::Published
        | TerminalPublicationState::Rejected { .. } | TerminalPublicationState::HistoryBlocked { .. } => {}
    }
    resident_inspection(
        snapshot, &child.request, Some(child.child_session_id.clone()), child.resumed_from.clone(), target,
    )
}

pub(in super::super) fn completed_inspection(
    child: &CompletedChild,
    persisted_output: Option<&str>,
    target: ResidentChildTarget,
    version: SourceVersion,
) -> Result<SubagentInspection, InventoryValidationError> {
    if child.result.subagent_id != child.request.id || child.result.child_session_id != child.child_session_id {
        return Err(InventoryValidationError::IdentityMismatch);
    }
    let mut inspection = completed_resident_inspection(child, persisted_output, target);
    match &child.publication {
        TerminalPublicationState::Committing | TerminalPublicationState::Publishing
        | TerminalPublicationState::Rejected { .. } | TerminalPublicationState::HistoryBlocked { .. } => {}
        TerminalPublicationState::Published => {
            let state = SubagentTerminalState::from_state(&child.result.state)
                .ok_or(InventoryValidationError::InvalidEvidence)?;
            let sources = vec![ChildEvidenceSource::Coordinator { version }];
            let recorded_state = match &state {
                SubagentTerminalState::Completed => RecordedTerminalState::Completed,
                SubagentTerminalState::Failed { message } => RecordedTerminalState::Failed {
                    message: Some(message.clone()),
                },
                SubagentTerminalState::Interrupted { cause } => RecordedTerminalState::Cancelled {
                    cause: Some(cause.clone()), message: None,
                },
            };
            let recorded = RecordedSubagentTerminal {
                attempt: child.attempt_evidence(), state: recorded_state,
                completed_at: None, duration_ms: Some(child.result.duration_ms),
                tool_calls: Some(child.result.tool_calls), turns: Some(child.result.turns),
                tokens_used: Some(child.result.tokens_used), output: None, sources: sources.clone(),
            };
            let terminal = match &child.origin {
                CompletedOrigin::NotStarted { .. } => {
                    let mut recorded = recorded;
                    if child.persisted_output_ref.is_none() || persisted_output.is_some() {
                        recorded.output = Some(persisted_output.map(Arc::from)
                            .unwrap_or_else(|| child.result.output.clone()));
                    }
                    ChildTerminalEvidence::Recorded { result: recorded }
                }
                CompletedOrigin::Attempt { .. } => {
                    let completion_id = SubagentCompletionId {
                        subagent_id: child.request.id.clone(), attempt_id: child.attempt_id.clone(),
                    };
                    if child.persisted_output_ref.is_none() || persisted_output.is_some() {
                        ChildTerminalEvidence::Producer {
                            completion_id,
                            result: SubagentTerminal::from_result(
                                &child.attempt_id, &child.result, persisted_output,
                            ).ok_or(InventoryValidationError::InvalidEvidence)?,
                            sources,
                        }
                    } else {
                        ChildTerminalEvidence::ProducerFact {
                            completion_id, state, recorded: Some(recorded), sources,
                        }
                    }
                }
            };
            inspection.observation = ChildObservation::Terminal { terminal };
        }
    }
    inspection.validate()?;
    Ok(inspection)
}
