use serde::{Deserialize, Serialize};
use xai_chat_state::SubagentCompletionId;
use xai_chat_state::source_version::SourceVersion;
use xai_message_delivery_core::AttemptId;

use super::terminal::ChildTerminalEvidence;
use super::types::{SubagentAccessError, SubagentInspection};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "attemptId", rename_all = "snake_case")]
pub enum AttemptEvidence {
    Known(#[serde(with = "xai_chat_state::completion::attempt_serde")] AttemptId),
    Legacy,
    NotStarted,
}

impl AttemptEvidence {
    pub fn from_recorded(attempt: Option<&str>) -> Result<Self, InventoryValidationError> {
        match attempt {
            Some(value) => AttemptId::parse(value)
                .map(Self::Known)
                .ok_or(InventoryValidationError::InvalidAttempt),
            None => Ok(Self::Legacy),
        }
    }

    pub fn known(&self) -> Option<&AttemptId> {
        match self {
            Self::Known(attempt) => Some(attempt),
            Self::Legacy | Self::NotStarted => None,
        }
    }
}

/// Recorded spawn provenance, not execution or resume authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordedSubagentOwner {
    Task,
    Workflow { run_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordedSubagentSpawner {
    Root,
    Child { subagent_id: String, session_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildIdentity {
    pub subagent_id: String,
    pub parent_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
}

impl ChildIdentity {
    pub fn validate(&self) -> Result<(), InventoryValidationError> {
        validate_id(&self.subagent_id)?;
        validate_id(&self.parent_session_id)?;
        if let Some(child) = &self.child_session_id {
            validate_id(child)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ChildEvidenceSource {
    ParentMeta {
        subagent_id: String,
        digest: String,
    },
    ParentJournal {
        line: u64,
        byte_offset: u64,
        byte_len: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_id: Option<String>,
        digest: String,
    },
    ParentSummary {
        digest: String,
        #[serde(with = "xai_chat_state::completion::completion_id_serde")]
        completion_id: SubagentCompletionId,
    },
    ChildSummary {
        child_session_id: String,
        digest: String,
    },
    Coordinator {
        version: SourceVersion,
    },
}

impl ChildEvidenceSource {
    pub fn validate(&self) -> Result<(), InventoryValidationError> {
        match self {
            Self::ParentMeta { subagent_id, digest } => {
                validate_id(subagent_id)?;
                validate_digest(digest)
            }
            Self::ParentJournal { line, byte_offset, byte_len, event_id, digest } => {
                if *line == 0 || *byte_len == 0 || byte_offset.checked_add(*byte_len).is_none() {
                    return Err(InventoryValidationError::InvalidSource);
                }
                if let Some(id) = event_id {
                    validate_id(id)?;
                }
                validate_digest(digest)
            }
            Self::ParentSummary { digest, completion_id } => {
                validate_id(&completion_id.subagent_id)?;
                validate_digest(digest)
            }
            Self::ChildSummary { child_session_id, digest } => {
                validate_id(child_session_id)?;
                validate_digest(digest)
            }
            Self::Coordinator { version } => validate_version(version),
        }
    }

    pub fn validate_for(
        &self,
        subagent_id: &str,
        attempt: &AttemptEvidence,
    ) -> Result<(), InventoryValidationError> {
        self.validate()?;
        match attempt {
            AttemptEvidence::Known(_) | AttemptEvidence::Legacy => {}
            AttemptEvidence::NotStarted => match self {
                Self::Coordinator { .. } => {}
                Self::ParentMeta { .. } | Self::ParentJournal { .. }
                | Self::ParentSummary { .. } | Self::ChildSummary { .. } => {
                    return Err(InventoryValidationError::InvalidAttempt);
                }
            },
        }
        match self {
            Self::ParentMeta { subagent_id: owner, .. } => {
                if owner != subagent_id {
                    return Err(InventoryValidationError::IdentityMismatch);
                }
            }
            Self::ParentSummary { completion_id, .. } => {
                if completion_id.subagent_id != subagent_id {
                    return Err(InventoryValidationError::IdentityMismatch);
                }
                if attempt.known() != Some(&completion_id.attempt_id) {
                    return Err(InventoryValidationError::AttemptMismatch);
                }
            }
            Self::ParentJournal { .. } | Self::ChildSummary { .. } | Self::Coordinator { .. } => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalChildPhase {
    Spawned,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ChildEvidence {
    Identity {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        child_session_id: Option<String>,
        description: String,
        subagent_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        started_at_epoch_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        persona: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resumed_from: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fork_parent_prompt_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recorded_owner: Option<RecordedSubagentOwner>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recorded_spawner: Option<RecordedSubagentSpawner>,
    },
    SummaryIdentity {
        child_session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fork_parent_prompt_id: Option<String>,
    },
    Nonterminal { phase: HistoricalChildPhase },
    Terminal { terminal: ChildTerminalEvidence },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildEvidenceRecord {
    pub subagent_id: String,
    pub attempt: AttemptEvidence,
    pub source: ChildEvidenceSource,
    pub value: ChildEvidence,
}

impl ChildEvidenceRecord {
    pub fn validate(&self) -> Result<(), InventoryValidationError> {
        validate_id(&self.subagent_id)?;
        self.source.validate_for(&self.subagent_id, &self.attempt)?;
        match &self.value {
            ChildEvidence::Identity { child_session_id, recorded_owner, recorded_spawner, .. } => {
                if let Some(id) = child_session_id {
                    validate_id(id)?;
                }
                let expected_child = match &self.source {
                    ChildEvidenceSource::ParentMeta { subagent_id, .. } => Some(subagent_id),
                    ChildEvidenceSource::ChildSummary { child_session_id, .. } => Some(child_session_id),
                    ChildEvidenceSource::ParentJournal { .. }
                    | ChildEvidenceSource::ParentSummary { .. }
                    | ChildEvidenceSource::Coordinator { .. } => None,
                };
                if let Some(expected) = expected_child
                    && child_session_id.as_ref() != Some(expected)
                {
                    return Err(InventoryValidationError::IdentityMismatch);
                }
                validate_owner(recorded_owner.as_ref())?;
                validate_spawner(recorded_spawner.as_ref())
            }
            ChildEvidence::SummaryIdentity { child_session_id, fork_parent_prompt_id } => {
                validate_id(child_session_id)?;
                if let Some(id) = fork_parent_prompt_id {
                    validate_id(id)?;
                }
                match &self.source {
                    ChildEvidenceSource::ChildSummary { child_session_id: source_child, .. }
                        if source_child == child_session_id && child_session_id == &self.subagent_id => Ok(()),
                    ChildEvidenceSource::ChildSummary { .. } => Err(InventoryValidationError::IdentityMismatch),
                    ChildEvidenceSource::ParentMeta { .. }
                    | ChildEvidenceSource::ParentJournal { .. }
                    | ChildEvidenceSource::ParentSummary { .. }
                    | ChildEvidenceSource::Coordinator { .. } => Err(InventoryValidationError::InvalidSource),
                }
            }
            ChildEvidence::Nonterminal { .. } => match &self.attempt {
                AttemptEvidence::Known(_) | AttemptEvidence::Legacy => Ok(()),
                AttemptEvidence::NotStarted => Err(InventoryValidationError::InvalidEvidence),
            },
            ChildEvidence::Terminal { terminal } => terminal.validate(&self.subagent_id, &self.attempt),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidentChildTarget {
    pub attempt: AttemptEvidence,
    pub incarnation: uuid::Uuid,
}

impl ResidentChildTarget {
    pub fn validate(&self) -> Result<(), InventoryValidationError> {
        if self.incarnation.is_nil() {
            return Err(InventoryValidationError::InvalidTarget);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum InventoryTarget {
    Resident {
        #[serde(flatten)]
        target: ResidentChildTarget,
    },
    Historical { attempt: AttemptEvidence, source: ChildEvidenceSource },
}

impl InventoryTarget {
    pub fn attempt(&self) -> &AttemptEvidence {
        match self {
            Self::Resident { target } => &target.attempt,
            Self::Historical { attempt, .. } => attempt,
        }
    }

    pub fn validate(&self, subagent_id: &str) -> Result<(), InventoryValidationError> {
        match self {
            Self::Resident { target } => target.validate(),
            Self::Historical { attempt, source } => source.validate_for(subagent_id, attempt),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCancelProof {
    pub version: SourceVersion,
    pub target: InventoryTarget,
}

impl InventoryCancelProof {
    pub fn validate(&self, subagent_id: &str) -> Result<(), InventoryValidationError> {
        validate_version(&self.version)?;
        self.target.validate(subagent_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildCancelSelector {
    Current,
    Bound(InventoryCancelProof),
    UnconfirmedObservation { attempt: AttemptEvidence },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryIoKind {
    NotFound,
    PermissionDenied,
    Interrupted,
    Other,
}

impl From<std::io::ErrorKind> for InventoryIoKind {
    fn from(kind: std::io::ErrorKind) -> Self {
        match kind {
            std::io::ErrorKind::NotFound => Self::NotFound,
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            std::io::ErrorKind::Interrupted => Self::Interrupted,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum InventoryReadSource {
    ParentDirectory,
    ParentMeta { subagent_id: String },
    ParentJournal,
    ParentSummary,
    ChildSummary { child_session_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum InventoryIssue {
    RuntimeCaptureUnavailable { subagent_id: String },
    Io { source: InventoryReadSource, error_kind: InventoryIoKind },
    Decode { source: InventoryReadSource },
    IdentityMismatch { source: ChildEvidenceSource },
    AttemptConflict { sources: Vec<ChildEvidenceSource> },
    OwnerConflict { sources: Vec<ChildEvidenceSource> },
    LineageConflict { sources: Vec<ChildEvidenceSource> },
    TerminalConflict { sources: Vec<ChildEvidenceSource> },
}

impl InventoryIssue {
    fn evidence_sources(&self) -> &[ChildEvidenceSource] {
        match self {
            Self::RuntimeCaptureUnavailable { .. } | Self::Io { .. } | Self::Decode { .. } => &[],
            Self::IdentityMismatch { source } => std::slice::from_ref(source),
            Self::AttemptConflict { sources }
            | Self::OwnerConflict { sources }
            | Self::LineageConflict { sources }
            | Self::TerminalConflict { sources } => sources,
        }
    }

    pub fn validate(&self) -> Result<(), InventoryValidationError> {
        match self {
            Self::RuntimeCaptureUnavailable { subagent_id } => validate_id(subagent_id),
            Self::Io { source, .. } | Self::Decode { source } => match source {
                InventoryReadSource::ParentMeta { subagent_id } => validate_id(subagent_id),
                InventoryReadSource::ChildSummary { child_session_id } => validate_id(child_session_id),
                InventoryReadSource::ParentDirectory
                | InventoryReadSource::ParentJournal
                | InventoryReadSource::ParentSummary => Ok(()),
            },
            Self::IdentityMismatch { source } => source.validate(),
            Self::AttemptConflict { sources }
            | Self::OwnerConflict { sources }
            | Self::LineageConflict { sources }
            | Self::TerminalConflict { sources } => {
                if sources.is_empty() {
                    return Err(InventoryValidationError::InvalidSource);
                }
                for source in sources {
                    source.validate()?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ChildInventoryEntry {
    Child {
        inspection: SubagentInspection,
        control_owner: Option<RecordedSubagentOwner>,
        #[serde(default)]
        issues: Vec<InventoryIssue>,
    },
    Reference {
        subagent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        child_session_id: Option<String>,
        records: Vec<ChildEvidenceRecord>,
    },
    Conflict { subagent_id: String, issues: Vec<InventoryIssue> },
}

impl ChildInventoryEntry {
    pub fn subagent_id(&self) -> &str {
        match self {
            Self::Child { inspection, .. } => &inspection.subagent_id,
            Self::Reference { subagent_id, .. } | Self::Conflict { subagent_id, .. } => subagent_id,
        }
    }

    pub fn validate(&self) -> Result<(), InventoryValidationError> {
        validate_id(self.subagent_id())?;
        match self {
            Self::Child { inspection, control_owner, issues } => {
                inspection.validate()?;
                validate_owner(control_owner.as_ref())?;
                for issue in issues {
                    issue.validate()?;
                }
            }
            Self::Reference { subagent_id, child_session_id, records } => {
                if let Some(id) = child_session_id {
                    validate_id(id)?;
                }
                if records.is_empty() {
                    return Err(InventoryValidationError::InvalidEvidence);
                }
                for record in records {
                    if &record.subagent_id != subagent_id {
                        return Err(InventoryValidationError::IdentityMismatch);
                    }
                    record.validate()?;
                }
            }
            Self::Conflict { issues, .. } => {
                if issues.is_empty() {
                    return Err(InventoryValidationError::InvalidEvidence);
                }
                for issue in issues {
                    issue.validate()?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildInventorySnapshot {
    pub parent_session_id: String,
    pub version: SourceVersion,
    pub entries: Vec<ChildInventoryEntry>,
}

impl ChildInventorySnapshot {
    pub fn validate(&self) -> Result<(), InventoryValidationError> {
        validate_id(&self.parent_session_id)?;
        validate_version(&self.version)?;
        let mut ids = std::collections::HashSet::new();
        for entry in &self.entries {
            entry.validate()?;
            if !ids.insert(entry.subagent_id()) {
                return Err(InventoryValidationError::IdentityMismatch);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ChildInventoryChange {
    Upsert { entry: ChildInventoryEntry },
    Remove { subagent_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildInventoryImport {
    pub parent_session_id: String,
    pub base: SourceVersion,
    pub records: Vec<ChildEvidenceRecord>,
    pub issues: Vec<InventoryIssue>,
}

impl ChildInventoryImport {
    pub fn validate(&self) -> Result<(), InventoryValidationError> {
        validate_id(&self.parent_session_id)?;
        validate_version(&self.base)?;
        for issue in &self.issues {
            issue.validate()?;
            for source in issue.evidence_sources() {
                reject_coordinator_source(source)?;
            }
        }
        for record in &self.records {
            record.validate()?;
            reject_coordinator_source(&record.source)?;
            if let ChildEvidence::Terminal { terminal } = &record.value {
                for source in terminal.sources() {
                    reject_coordinator_source(source)?;
                }
                if let Some(recorded) = terminal.recorded() {
                    for source in &recorded.sources {
                        reject_coordinator_source(source)?;
                    }
                }
            }
        }
        Ok(())
    }
}

pub type ChildInventoryReply = Result<Option<ChildInventoryEntry>, SubagentAccessError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InventoryValidationError {
    #[error("invalid child identity")]
    InvalidIdentity,
    #[error("invalid child attempt")]
    InvalidAttempt,
    #[error("invalid child evidence source")]
    InvalidSource,
    #[error("child evidence identity mismatch")]
    IdentityMismatch,
    #[error("child evidence attempt mismatch")]
    AttemptMismatch,
    #[error("invalid inventory target observation")]
    InvalidTarget,
    #[error("invalid child evidence")]
    InvalidEvidence,
    #[error("runtime provenance cannot be imported from disk")]
    RuntimeSourceInImport,
}

pub(super) fn validate_sources(
    sources: &[ChildEvidenceSource],
    subagent_id: &str,
    attempt: &AttemptEvidence,
) -> Result<(), InventoryValidationError> {
    validate_id(subagent_id)?;
    if sources.is_empty() {
        return Err(InventoryValidationError::InvalidSource);
    }
    for source in sources {
        source.validate_for(subagent_id, attempt)?;
    }
    Ok(())
}

pub(super) fn validate_owner(owner: Option<&RecordedSubagentOwner>) -> Result<(), InventoryValidationError> {
    match owner {
        Some(RecordedSubagentOwner::Workflow { run_id }) => validate_id(run_id),
        Some(RecordedSubagentOwner::Task) | None => Ok(()),
    }
}

pub(super) fn validate_spawner(spawner: Option<&RecordedSubagentSpawner>) -> Result<(), InventoryValidationError> {
    match spawner {
        Some(RecordedSubagentSpawner::Child { subagent_id, session_id }) => {
            validate_id(subagent_id)?;
            validate_id(session_id)
        }
        Some(RecordedSubagentSpawner::Root) | None => Ok(()),
    }
}

pub(super) fn validate_id(id: &str) -> Result<(), InventoryValidationError> {
    if id.is_empty() || id.contains('\0') {
        return Err(InventoryValidationError::InvalidIdentity);
    }
    Ok(())
}

pub(super) fn validate_version(version: &SourceVersion) -> Result<(), InventoryValidationError> {
    if version.incarnation.is_nil() {
        return Err(InventoryValidationError::InvalidSource);
    }
    Ok(())
}

fn validate_digest(digest: &str) -> Result<(), InventoryValidationError> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(InventoryValidationError::InvalidSource);
    }
    Ok(())
}

fn reject_coordinator_source(source: &ChildEvidenceSource) -> Result<(), InventoryValidationError> {
    match source {
        ChildEvidenceSource::Coordinator { .. } => Err(InventoryValidationError::RuntimeSourceInImport),
        ChildEvidenceSource::ParentMeta { .. }
        | ChildEvidenceSource::ParentJournal { .. }
        | ChildEvidenceSource::ParentSummary { .. }
        | ChildEvidenceSource::ChildSummary { .. } => Ok(()),
    }
}

#[cfg(test)]
#[path = "inventory_tests.rs"]
mod tests;
