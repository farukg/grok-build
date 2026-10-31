//! Parent-local, read-only child evidence boundaries.
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use xai_chat_state::source_version::SourceVersion;
use xai_grok_tools::implementations::grok_build::task::inventory::*;
use xai_grok_tools::implementations::grok_build::task::terminal::*;
use xai_grok_tools::implementations::grok_build::task::types::SubagentAccessError;
use crate::extensions::notification::{SessionNotification, SessionUpdate};
use crate::session::completion_delivery::{SubagentCompletionEvent, SubagentCompletionFact};
use super::{SubagentMeta, SubagentMetaStatus, metadata_store};

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn io_issue(source: InventoryReadSource, error: &io::Error) -> InventoryIssue {
    let error_kind = match error.kind() {
        io::ErrorKind::NotFound => InventoryIoKind::NotFound,
        io::ErrorKind::PermissionDenied => InventoryIoKind::PermissionDenied,
        io::ErrorKind::Interrupted => InventoryIoKind::Interrupted,
        _ => InventoryIoKind::Other,
    };
    InventoryIssue::Io { source, error_kind }
}

fn recorded_state(status: SubagentMetaStatus, message: Option<String>) -> Option<RecordedTerminalState> {
    match status {
        SubagentMetaStatus::Running => None,
        SubagentMetaStatus::Completed => Some(RecordedTerminalState::Completed),
        SubagentMetaStatus::Failed => Some(RecordedTerminalState::Failed { message }),
        SubagentMetaStatus::Cancelled => Some(RecordedTerminalState::Cancelled { cause: None, message }),
    }
}

fn meta_records(meta: &SubagentMeta, bytes: &[u8]) -> Result<Vec<ChildEvidenceRecord>, InventoryValidationError> {
    let attempt = AttemptEvidence::from_recorded(meta.attempt_id.as_deref())?;
    let source = ChildEvidenceSource::ParentMeta { subagent_id: meta.subagent_id.clone(), digest: digest(bytes) };
    let record = |value| ChildEvidenceRecord {
        subagent_id: meta.subagent_id.clone(), attempt: attempt.clone(), source: source.clone(), value,
    };
    let identity = record(ChildEvidence::Identity {
        child_session_id: Some(meta.child_session_id.clone()),
        description: meta.description.clone(), subagent_type: meta.subagent_type.clone(),
        started_at_epoch_ms: u64::try_from(meta.started_at.timestamp_millis()).ok(),
        persona: meta.persona.clone(), resumed_from: meta.resumed_from.clone(), fork_parent_prompt_id: None,
        recorded_owner: meta.recorded_owner.clone(), recorded_spawner: meta.recorded_spawner.clone(),
    });
    identity.validate()?;
    let value = match recorded_state(meta.status, meta.error.clone()) {
        None => ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Running },
        Some(state) => ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded {
            result: RecordedSubagentTerminal {
                attempt: attempt.clone(), state, completed_at: meta.completed_at,
                duration_ms: meta.duration_ms, tool_calls: meta.tool_calls, turns: meta.turns,
                tokens_used: None, output: None, sources: vec![source.clone()],
            },
        } },
    };
    Ok(vec![identity, record(value)])
}

/// Consumes evidence once; replay output and highwater stay with the load owner.
pub(crate) async fn prepare_parent_inventory(
    parent: &Path,
    parent_session_id: &str,
    base: SourceVersion,
    replay: &mut crate::session::storage::PreparedReplay<'_>,
    parent_summary_bytes: &[u8],
) -> ChildInventoryImport {
    let mut records = std::mem::take(&mut replay.child_evidence);
    let mut issues = std::mem::take(&mut replay.child_evidence_issues);
    match parent_summary_records(parent_summary_bytes, parent_session_id) {
        Ok(found) => records.extend(found),
        Err(_) => issues.push(InventoryIssue::Decode { source: InventoryReadSource::ParentSummary }),
    }
    read_parent_inventory(parent, parent_session_id, base, records, issues).await
}

/// The caller owns the single replay preparation and its load fence.
pub(crate) async fn read_parent_inventory(
    parent: &Path,
    parent_session_id: &str,
    base: SourceVersion,
    mut records: Vec<ChildEvidenceRecord>,
    mut issues: Vec<InventoryIssue>,
) -> ChildInventoryImport {
    let owned_parent = parent.to_owned();
    let owned_session = parent_session_id.to_owned();
    let read = tokio::task::spawn_blocking(move || {
        let (metas, issues) = metadata_store::enumerate_owned(&owned_parent, &owned_session);
        let metas = metas.into_iter().map(|(meta, bytes)| {
            let summary = read_child_summary(&owned_parent, &meta);
            (meta, bytes, summary)
        }).collect::<Vec<_>>();
        (metas, issues)
    }).await;
    let (metas, errors) = match read {
        Ok(read) => read,
        Err(_) => return ChildInventoryImport {
            parent_session_id: parent_session_id.to_owned(), base, records,
            issues: {
                issues.push(InventoryIssue::Io { source: InventoryReadSource::ParentDirectory, error_kind: InventoryIoKind::Other });
                issues
            },
        },
    };
    issues.extend(errors);
    for (meta, bytes, summary) in metas {
        match meta_records(&meta, &bytes) {
            Ok(found) => {
                records.extend(found);
                match summary {
                    Ok(Some((summary, summary_bytes))) => {
                        let source = ChildEvidenceSource::ChildSummary {
                            child_session_id: meta.child_session_id.clone(), digest: digest(&summary_bytes),
                        };
                        if summary.parent_session_id.as_deref() != Some(parent_session_id)
                            || summary.info.id.0.as_ref() != meta.child_session_id
                        {
                            issues.push(InventoryIssue::IdentityMismatch { source });
                            continue;
                        }
                        match AttemptEvidence::from_recorded(summary.attempt_id.as_deref()) {
                            Ok(attempt) => {
                                if AttemptEvidence::from_recorded(meta.attempt_id.as_deref()).as_ref() != Ok(&attempt) {
                                    issues.push(InventoryIssue::AttemptConflict { sources: vec![
                                        ChildEvidenceSource::ParentMeta {
                                            subagent_id: meta.subagent_id.clone(), digest: digest(&bytes),
                                        },
                                        source,
                                    ] });
                                }
                            }
                            Err(_) => issues.push(InventoryIssue::Decode { source: InventoryReadSource::ChildSummary {
                                child_session_id: meta.child_session_id.clone(),
                            } }),
                        }
                    }
                    Ok(None) => {}
                    Err(error) => issues.push(io_issue(InventoryReadSource::ChildSummary {
                        child_session_id: meta.child_session_id.clone(),
                    }, &error)),
                }
            },
            Err(_) => issues.push(InventoryIssue::Decode {
                source: InventoryReadSource::ParentMeta { subagent_id: meta.subagent_id },
            }),
        }
    }
    ChildInventoryImport { parent_session_id: parent_session_id.to_owned(), base, records, issues }
}

fn read_child_summary(
    parent: &Path, meta: &SubagentMeta,
) -> io::Result<Option<(crate::session::persistence::Summary, Vec<u8>)>> {
    let Some(cwd) = meta.child_cwd.as_ref() else { return Ok(None); };
    let sessions = parent.parent().and_then(Path::parent)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing sessions directory"))?;
    let encoded = xai_grok_config::encode_cwd_dirname(cwd);
    let path = sessions.join(encoded).join(&meta.child_session_id).join("summary.json");
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let summary = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid child summary"))?;
    Ok(Some((summary, bytes)))
}

pub(crate) struct InventorySourceContext {
    pub(crate) metadata: SubagentMeta,
    pub(crate) summary: Option<crate::session::persistence::Summary>,
}

/// Context only: admission, continuation, and runtime custody remain with their owners.
pub(crate) async fn load_inventory_source_context(
    parent: &Path, parent_session_id: &str, subagent_id: &str,
    attempt: &AttemptEvidence, source: &ChildEvidenceSource,
) -> Result<InventorySourceContext, SubagentAccessError> {
    let target = InventoryReadTarget::new(parent, parent_session_id, subagent_id, attempt, source);
    tokio::task::spawn_blocking(move || load_inventory_source_context_at(
        &target.parent, &target.parent_session_id, &target.subagent_id, &target.attempt, &target.source,
    )).await.map_err(|_| SubagentAccessError::InventoryUnreadable {
        issues: vec![InventoryIssue::Io { source: InventoryReadSource::ParentDirectory, error_kind: InventoryIoKind::Other }],
    })?
}

fn load_inventory_source_context_at(
    parent: &Path, parent_session_id: &str, subagent_id: &str,
    attempt: &AttemptEvidence, source: &ChildEvidenceSource,
) -> Result<InventorySourceContext, SubagentAccessError> {
    source.validate_for(subagent_id, attempt).map_err(invalid)?;
    let (meta, bytes) = metadata_store::read_owned_bytes(parent, subagent_id, parent_session_id)
        .map_err(|error| SubagentAccessError::InventoryUnreadable { issues: vec![io_issue(
            InventoryReadSource::ParentMeta { subagent_id: subagent_id.to_owned() }, &error)] })?;
    if AttemptEvidence::from_recorded(meta.attempt_id.as_deref()).map_err(invalid)? != *attempt {
        return Err(SubagentAccessError::InventoryChanged);
    }
    match source {
        ChildEvidenceSource::ParentMeta { digest: expected, .. } => {
            if digest(&bytes) != *expected { return Err(SubagentAccessError::InventoryChanged); }
        }
        ChildEvidenceSource::ParentJournal { .. } | ChildEvidenceSource::ParentSummary { .. } => {
            load_inventory_terminal_at(parent, parent_session_id, subagent_id, attempt, source)?;
        }
        ChildEvidenceSource::ChildSummary { .. } => {}
        ChildEvidenceSource::Coordinator { .. } => return Err(invalid(InventoryValidationError::InvalidSource)),
    }
    let summary = read_child_summary(parent, &meta).map_err(|error| SubagentAccessError::InventoryUnreadable {
        issues: vec![io_issue(InventoryReadSource::ChildSummary { child_session_id: meta.child_session_id.clone() }, &error)],
    })?;
    if let Some((summary, bytes)) = &summary {
        if summary.info.id.0.as_ref() != meta.child_session_id
            || summary.parent_session_id.as_deref() != Some(parent_session_id)
            || AttemptEvidence::from_recorded(summary.attempt_id.as_deref()).map_err(invalid)? != *attempt
        { return Err(SubagentAccessError::InventoryChanged); }
        if let ChildEvidenceSource::ChildSummary { digest: expected, child_session_id } = source {
            if digest(bytes) != *expected || *child_session_id != meta.child_session_id {
                return Err(SubagentAccessError::InventoryChanged);
            }
        }
    } else if matches!(source, ChildEvidenceSource::ChildSummary { .. }) {
        return Err(SubagentAccessError::InventoryChanged);
    }
    Ok(InventorySourceContext { metadata: meta, summary: summary.map(|(summary, _)| summary) })
}

#[derive(serde::Deserialize)]
struct RawRecord<'a> {
    #[serde(borrow)]
    params: Option<&'a serde_json::value::RawValue>,
}

#[derive(serde::Deserialize)]
struct RawNotification<'a> {
    #[serde(borrow)]
    update: &'a serde_json::value::RawValue,
}

#[derive(serde::Deserialize)]
struct ReportedTokens {
    tokens_used: Option<u64>,
}

/// Shares the existing notification decoder; optional wire fields stay optional.
pub(crate) fn journal_records(
    bytes: &str,
    source: ChildEvidenceSource,
    parent_session_id: Option<&str>,
) -> Result<Vec<ChildEvidenceRecord>, InventoryValidationError> {
    fn decode<E>(_: E) -> InventoryValidationError { InventoryValidationError::InvalidEvidence }
    let envelope: RawRecord<'_> = serde_json::from_str(bytes).map_err(decode)?;
    let raw = envelope.params.map_or(bytes, |params| params.get());
    let notification: SessionNotification = serde_json::from_str(raw).map_err(decode)?;
    if parent_session_id.is_some_and(|parent| notification.session_id.0.as_ref() != parent) {
        return Err(InventoryValidationError::IdentityMismatch);
    }
    if let ChildEvidenceSource::ParentJournal { event_id, .. } = &source {
        let recorded_id = notification.meta.as_ref().and_then(|meta| meta.get("eventId")).and_then(|id| id.as_str());
        if recorded_id != event_id.as_deref() { return Err(InventoryValidationError::InvalidSource); }
    }
    let fact = notification.meta.as_ref().and_then(|meta| meta.get("subagentCompletion"))
        .map(|value| serde_json::from_value::<SubagentCompletionFact>(value.clone()))
        .transpose().map_err(decode)?;
    let mut records = Vec::new();
    if let SessionUpdate::SubagentSpawned {
            subagent_id, attempt_id, parent_session_id: recorded_parent, child_session_id,
            subagent_type, description, persona, resumed_from, ..
        } = notification.update {
            if parent_session_id.is_some_and(|parent| parent != recorded_parent) {
                return Err(InventoryValidationError::IdentityMismatch);
            }
            let attempt = AttemptEvidence::from_recorded(attempt_id.as_deref())?;
            records.push(ChildEvidenceRecord {
                subagent_id: subagent_id.clone(), attempt: attempt.clone(), source: source.clone(),
                value: ChildEvidence::Identity {
                    child_session_id: Some(child_session_id), description, subagent_type,
                    started_at_epoch_ms: None, persona, resumed_from, fork_parent_prompt_id: None,
                    recorded_owner: None, recorded_spawner: None,
                },
            });
            records.push(ChildEvidenceRecord { subagent_id, attempt, source,
                value: ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Spawned } });
        } else if let SessionUpdate::SubagentFinished {
            subagent_id, attempt_id, child_session_id, status, error, tool_calls, turns, duration_ms, output, ..
        } = notification.update {
            if child_session_id != subagent_id { return Err(InventoryValidationError::IdentityMismatch); }
            let attempt = AttemptEvidence::from_recorded(attempt_id.as_deref())?;
            let status = status.parse::<SubagentMetaStatus>().map_err(decode)?;
            let state = recorded_state(status, error).ok_or(InventoryValidationError::InvalidEvidence)?;
            let raw_notification: RawNotification<'_> = serde_json::from_str(raw).map_err(decode)?;
            let reported: ReportedTokens = serde_json::from_str(raw_notification.update.get()).map_err(decode)?;
            let recorded = RecordedSubagentTerminal {
                attempt: attempt.clone(), state, completed_at: None, duration_ms: Some(duration_ms),
                tool_calls: Some(tool_calls), turns: Some(turns), tokens_used: reported.tokens_used,
                output: output.map(Arc::from), sources: vec![source.clone()],
            };
            let terminal = match fact {
                Some(fact) => {
                    if fact.id.subagent_id != subagent_id || attempt.known() != Some(&fact.id.attempt_id) {
                        return Err(InventoryValidationError::AttemptMismatch);
                    }
                    let state = SubagentTerminalState::from_state(&fact.state)
                        .ok_or(InventoryValidationError::InvalidEvidence)?;
                    let candidate = ChildTerminalEvidence::ProducerFact {
                        completion_id: fact.id.clone(), state: state.clone(),
                        recorded: Some(recorded.clone()), sources: vec![source.clone()],
                    };
                    let compatible = candidate.validate(&subagent_id, &attempt).is_ok();
                    if !compatible {
                        records.push(ChildEvidenceRecord {
                            subagent_id: subagent_id.clone(), attempt: attempt.clone(), source: source.clone(),
                            value: ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Recorded { result: recorded.clone() } },
                        });
                    }
                    match (&recorded.output, recorded.tokens_used, compatible) {
                        (Some(output), Some(tokens_used), true) => ChildTerminalEvidence::Producer {
                            completion_id: fact.id,
                            result: SubagentTerminal { attempt_id: attempt.known()
                                .ok_or(InventoryValidationError::InvalidAttempt)?.to_string(), state,
                                output: output.clone(), tool_calls, turns, duration_ms, tokens_used },
                            sources: vec![source.clone()],
                        },
                        _ => ChildTerminalEvidence::ProducerFact {
                            completion_id: fact.id, state, recorded: compatible.then_some(recorded), sources: vec![source.clone()],
                        },
                    }
                }
                None => ChildTerminalEvidence::Recorded { result: recorded },
            };
            terminal.validate(&subagent_id, &attempt)?;
            records.push(ChildEvidenceRecord { subagent_id, attempt, source,
                value: ChildEvidence::Terminal { terminal } });
        } else {
            return Err(InventoryValidationError::InvalidEvidence);
        }
    for record in &records { record.validate()?; }
    Ok(records)
}

pub(crate) fn producer_record(
    event: &SubagentCompletionEvent,
    source: ChildEvidenceSource,
) -> Result<ChildEvidenceRecord, InventoryValidationError> {
    if event.child_session_id != event.id.subagent_id {
        return Err(InventoryValidationError::IdentityMismatch);
    }
    let attempt = AttemptEvidence::Known(event.id.attempt_id.clone());
    let state = SubagentTerminalState::from_state(&event.state)
        .ok_or(InventoryValidationError::InvalidEvidence)?;
    let record = ChildEvidenceRecord {
        subagent_id: event.id.subagent_id.clone(), attempt, source: source.clone(),
        value: ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Producer {
            completion_id: event.id.clone(),
            result: SubagentTerminal { attempt_id: event.id.attempt_id.to_string(), state,
                output: Arc::from(event.output.as_str()), tool_calls: event.tool_calls,
                turns: event.turns, duration_ms: event.duration_ms, tokens_used: event.tokens_used },
            sources: vec![source],
        } },
    };
    record.validate()?;
    Ok(record)
}

#[derive(serde::Deserialize)]
struct RawSummary<'a> {
    info: crate::session::info::Info,
    #[serde(default, borrow)]
    completion_delivery: RawDelivery<'a>,
}

#[derive(Default, serde::Deserialize)]
struct RawDelivery<'a> {
    #[serde(default, borrow)]
    obligations: Vec<&'a serde_json::value::RawValue>,
}

#[derive(serde::Deserialize)]
struct RawPreparing<'a> {
    #[serde(borrow)]
    event: &'a serde_json::value::RawValue,
}

/// Hashes the embedded event bytes, not the mutable enclosing summary.
pub(crate) fn parent_summary_records(
    bytes: &[u8], parent_session_id: &str,
) -> Result<Vec<ChildEvidenceRecord>, InventoryValidationError> {
    use crate::session::completion_delivery::CompletionObligation;
    let raw: RawSummary<'_> = serde_json::from_slice(bytes)
        .map_err(|_| InventoryValidationError::InvalidEvidence)?;
    if raw.info.id.0.as_ref() != parent_session_id {
        return Err(InventoryValidationError::IdentityMismatch);
    }
    let mut records = Vec::new();
    for obligation in raw.completion_delivery.obligations {
        let parsed: CompletionObligation = serde_json::from_str(obligation.get())
            .map_err(|_| InventoryValidationError::InvalidEvidence)?;
        match parsed {
            CompletionObligation::Preparing { event, .. } => {
                let raw: RawPreparing<'_> = serde_json::from_str(obligation.get())
                    .map_err(|_| InventoryValidationError::InvalidEvidence)?;
                let source = ChildEvidenceSource::ParentSummary {
                    digest: digest(raw.event.get().as_bytes()), completion_id: event.id.clone(),
                };
                records.push(producer_record(&event, source)?);
            }
            CompletionObligation::Pending { .. } | CompletionObligation::Projected { .. }
            | CompletionObligation::Suppressed { .. } => {}
        }
    }
    Ok(records)
}

#[derive(serde::Deserialize)]
struct PointSummary<'a> {
    info: crate::session::info::Info,
    #[serde(default, borrow)]
    completion_delivery: PointDelivery<'a>,
}

#[derive(Default, serde::Deserialize)]
struct PointDelivery<'a> {
    #[serde(default, borrow)]
    obligations: Vec<PointObligation<'a>>,
}

enum PointObligation<'a> {
    Preparing {
        event: &'a serde_json::value::RawValue,
        admission: &'a serde_json::value::RawValue,
    },
    Other,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ObligationPhase { Preparing, Pending, Projected, Suppressed }

impl<'de: 'a, 'a> serde::Deserialize<'de> for PointObligation<'a> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObligationVisitor;
        impl<'de> serde::de::Visitor<'de> for ObligationVisitor {
            type Value = PointObligation<'de>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a completion obligation")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut phase = None;
                let mut event = None;
                let mut admission = None;
                while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
                    match key.as_ref() {
                        "phase" => {
                            if phase.is_some() { return Err(serde::de::Error::duplicate_field("phase")); }
                            phase = Some(map.next_value::<ObligationPhase>()?);
                        }
                        "event" => {
                            if event.is_some() { return Err(serde::de::Error::duplicate_field("event")); }
                            event = Some(map.next_value::<&'de serde_json::value::RawValue>()?);
                        }
                        "admission" => {
                            if admission.is_some() { return Err(serde::de::Error::duplicate_field("admission")); }
                            admission = Some(map.next_value::<&'de serde_json::value::RawValue>()?);
                        }
                        _ => { map.next_value::<serde::de::IgnoredAny>()?; }
                    }
                }
                match phase.ok_or_else(|| serde::de::Error::missing_field("phase"))? {
                    ObligationPhase::Preparing => Ok(PointObligation::Preparing {
                        event: event.ok_or_else(|| serde::de::Error::missing_field("event"))?,
                        admission: admission.ok_or_else(|| serde::de::Error::missing_field("admission"))?,
                    }),
                    ObligationPhase::Pending | ObligationPhase::Projected | ObligationPhase::Suppressed => Ok(PointObligation::Other),
                }
            }
        }
        deserializer.deserialize_map(ObligationVisitor)
    }
}

// Summary has no byte index: traverse its JSON once, borrowing unrelated event payloads.
fn parent_summary_record(
    bytes: &[u8], parent_session_id: &str, subagent_id: &str,
    attempt: &AttemptEvidence, source: &ChildEvidenceSource,
) -> Result<ChildEvidenceRecord, SubagentAccessError> {
    let ChildEvidenceSource::ParentSummary { digest: expected, completion_id } = source else {
        return Err(invalid(InventoryValidationError::InvalidSource));
    };
    let raw: PointSummary<'_> = serde_json::from_slice(bytes)
        .map_err(|_| invalid(InventoryValidationError::InvalidEvidence))?;
    if raw.info.id.0.as_ref() != parent_session_id {
        return Err(invalid(InventoryValidationError::IdentityMismatch));
    }
    for obligation in raw.completion_delivery.obligations {
        match obligation {
            PointObligation::Preparing { event, admission } => {
                if digest(event.get().as_bytes()) != *expected { continue; }
                let event: SubagentCompletionEvent = serde_json::from_str(event.get())
                    .map_err(|_| invalid(InventoryValidationError::InvalidEvidence))?;
                let _: crate::session::completion_delivery::CompletionAdmission = serde_json::from_str(admission.get())
                    .map_err(|_| invalid(InventoryValidationError::InvalidEvidence))?;
                if event.id != *completion_id || event.id.subagent_id != subagent_id {
                    return Err(invalid(InventoryValidationError::IdentityMismatch));
                }
                let record = producer_record(&event, source.clone()).map_err(invalid)?;
                if record.attempt != *attempt {
                    return Err(invalid(InventoryValidationError::AttemptMismatch));
                }
                return Ok(record);
            }
            PointObligation::Other => {}
        }
    }
    Err(SubagentAccessError::InventoryChanged)
}

fn invalid(reason: InventoryValidationError) -> SubagentAccessError {
    SubagentAccessError::InvalidInventory { reason }
}

/// Reads only the selected record; never enumerates, recovers, or starts a child.
pub(crate) async fn load_inventory_terminal(
    parent: &Path,
    parent_session_id: &str,
    subagent_id: &str,
    attempt: &AttemptEvidence,
    source: &ChildEvidenceSource,
) -> Result<Option<ChildTerminalEvidence>, SubagentAccessError> {
    let target = InventoryReadTarget::new(parent, parent_session_id, subagent_id, attempt, source);
    tokio::task::spawn_blocking(move || load_inventory_terminal_at(
        &target.parent, &target.parent_session_id, &target.subagent_id, &target.attempt, &target.source,
    )).await.map_err(|_| SubagentAccessError::InventoryUnreadable {
        issues: vec![InventoryIssue::Io { source: InventoryReadSource::ParentDirectory, error_kind: InventoryIoKind::Other }],
    })?
}

struct InventoryReadTarget {
    parent: std::path::PathBuf,
    parent_session_id: String,
    subagent_id: String,
    attempt: AttemptEvidence,
    source: ChildEvidenceSource,
}

impl InventoryReadTarget {
    fn new(parent: &Path, parent_session_id: &str, subagent_id: &str,
        attempt: &AttemptEvidence, source: &ChildEvidenceSource) -> Self {
        Self { parent: parent.to_owned(), parent_session_id: parent_session_id.to_owned(),
            subagent_id: subagent_id.to_owned(), attempt: attempt.clone(), source: source.clone() }
    }
}

fn load_inventory_terminal_at(
    parent: &Path, parent_session_id: &str, subagent_id: &str,
    attempt: &AttemptEvidence, source: &ChildEvidenceSource,
) -> Result<Option<ChildTerminalEvidence>, SubagentAccessError> {
    source.validate_for(subagent_id, attempt).map_err(invalid)?;
    let records = match source {
        ChildEvidenceSource::ParentMeta { digest: expected, .. } => {
            let (meta, bytes) = metadata_store::read_owned_bytes(parent, subagent_id, parent_session_id)
                .map_err(|error| SubagentAccessError::InventoryUnreadable { issues: vec![io_issue(
                    InventoryReadSource::ParentMeta { subagent_id: subagent_id.to_owned() }, &error)] })?;
            if digest(&bytes) != *expected { return Err(SubagentAccessError::InventoryChanged); }
            meta_records(&meta, &bytes).map_err(invalid)?
        }
        ChildEvidenceSource::ParentJournal { byte_offset, byte_len, digest: expected, .. } => {
            let read = || -> io::Result<String> {
                let mut file = std::fs::File::open(parent.join("updates.jsonl"))?;
                file.seek(SeekFrom::Start(*byte_offset))?;
                let len = usize::try_from(*byte_len).map_err(io::Error::other)?;
                if *byte_len > file.metadata()?.len().saturating_sub(*byte_offset) {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "inventory record truncated"));
                }
                let mut bytes = vec![0; len];
                file.read_exact(&mut bytes)?;
                String::from_utf8(bytes).map_err(io::Error::other)
            };
            let bytes = read().map_err(|error| SubagentAccessError::InventoryUnreadable {
                issues: vec![io_issue(InventoryReadSource::ParentJournal, &error)],
            })?;
            if digest(bytes.as_bytes()) != *expected { return Err(SubagentAccessError::InventoryChanged); }
            journal_records(&bytes, source.clone(), Some(parent_session_id)).map_err(invalid)?
        }
        ChildEvidenceSource::ParentSummary { .. } => {
            let bytes = std::fs::read(parent.join("summary.json")).map_err(|error| {
                SubagentAccessError::InventoryUnreadable {
                    issues: vec![io_issue(InventoryReadSource::ParentSummary, &error)],
                }
            })?;
            vec![parent_summary_record(&bytes, parent_session_id, subagent_id, attempt, source)?]
        }
        ChildEvidenceSource::ChildSummary { .. } => {
            load_inventory_source_context_at(parent, parent_session_id, subagent_id, attempt, source)?;
            return Ok(None);
        }
        ChildEvidenceSource::Coordinator { .. } => return Err(invalid(InventoryValidationError::InvalidSource)),
    };
    for record in records.into_iter().rev() {
        if record.subagent_id != subagent_id || record.attempt != *attempt {
            return Err(invalid(InventoryValidationError::AttemptMismatch));
        }
        if let ChildEvidence::Terminal { terminal } = record.value { return Ok(Some(terminal)); }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "inventory_tests.rs"]
mod tests;
