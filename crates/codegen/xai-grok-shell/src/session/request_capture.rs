use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use xai_grok_sampler::request_capture::{LocalAttemptOutcome, RequestAttemptId};
use xai_grok_sampling_types::{ApiBackend, ContentEvidence, ProjectedRequestMember, SourceOccurrence, SourceOccurrenceId};

pub const REQUEST_SNAPSHOT_NOTIFICATION: &str = "x.ai/session/request_snapshot/observed";
pub const SOURCE_PUBLICATION_META: &str = "requestSource";
pub const INPUT_SOURCE_NOTIFICATION: &str = "x.ai/session/request_source/applied";
pub const INPUT_SOURCE_META: &str = "requestInput";

/// All echo blocks of one input; not a per-block provider item identity.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptInputRef {
    pub session_id: String,
    pub prompt_id: String,
    pub turn: u64,
    pub generation: u64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedSourceMeta {
    #[serde(rename = "requestSource")]
    pub publication: SourcePublication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSourceUnknown {
    StrictReceiptSourceUnavailable,
    NotApplied,
    NotCommitted { cause: CaptureFailure },
    Indeterminate { cause: CaptureFailure },
    MailboxClosed,
    /// The append may already have applied.
    ReplyLost,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "binding", rename_all = "snake_case")]
pub enum InputSourceBinding {
    Applied { #[serde(rename = "_meta")] meta: AppliedSourceMeta },
    Unknown { cause: InputSourceUnknown },
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct InputSourceNotification {
    pub input: PromptInputRef,
    #[serde(flatten)]
    pub binding: InputSourceBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestSnapshotId(pub uuid::Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestOrigin {
    pub session_id: String,
    pub request: RequestSnapshotId,
    pub turn: u64,
    pub generation: u64,
    pub prompt_id: Option<String>,
    pub conversation_id: Option<String>,
    pub logical_request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSnapshotRef {
    pub session_id: String,
    pub request: RequestSnapshotId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceDurability {
    UnconfirmedIndependentHistoryChannel,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SourcePublication {
    pub source: SourceOccurrence,
    pub evidence: ContentEvidence,
    pub durability: SourceDurability,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BodyAvailability {
    Captured { publication_digest: [u8; 32], representation: xai_grok_sampler::request_capture::PayloadRepresentation },
    Expired,
    NotCaptured { reason: NotCapturedReason },
    Failed { cause: CaptureFailure },
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedBody {
    pub digest: [u8; 32],
    pub byte_len: u64,
    pub availability: BodyAvailability,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotCapturedReason { Oversize }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureFailure {
    Storage,
    Io { kind: crate::extensions::context_policy::ContextIoKind, os_code: Option<i32> },
    ReplyLost,
    MailboxClosed,
    OriginUnavailable,
    InvalidEvidence,
    WorkerLost,
}

impl CaptureFailure {
    pub(crate) fn io(error: &std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::InvalidData { Self::InvalidEvidence }
        else { Self::Io { kind: error.kind().into(), os_code: error.raw_os_error() } }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome { Completed, Empty, Failed, Cancelled, InitializationFailed }

impl From<LocalAttemptOutcome> for AttemptOutcome {
    fn from(value: LocalAttemptOutcome) -> Self {
        match value {
            LocalAttemptOutcome::Completed => Self::Completed,
            LocalAttemptOutcome::Empty => Self::Empty,
            LocalAttemptOutcome::Failed => Self::Failed,
            LocalAttemptOutcome::Cancelled => Self::Cancelled,
            LocalAttemptOutcome::InitializationFailed => Self::InitializationFailed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RequestObservation {
    Prepared {
        backend: ApiBackend,
        membership: Vec<ProjectedRequestMember>,
        body: PreparedBody,
        source_durability: SourceDurability,
    },
    DispatchStarted,
    ResponseHeaders { status: u16 },
    Finished { outcome: AttemptOutcome },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptObservation {
    pub attempt: RequestAttemptId,
    pub observation: RequestObservation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnobservedRequestPurpose {
    Compaction, Title, Recap, Summary, MemoryDreamV1, MemoryDreamV2,
    MemoryDreamBatch, Goal, Laziness, ImageDescribe, PermissionClassifier,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "coverage", rename_all = "snake_case")]
pub enum RequestCaptureScope {
    MainTurnOnly { unobserved: Vec<UnobservedRequestPurpose> },
}

impl Default for RequestCaptureScope {
    fn default() -> Self {
        use UnobservedRequestPurpose::*;
        Self::MainTurnOnly { unobserved: vec![Compaction, Title, Recap, Summary, MemoryDreamV1,
            MemoryDreamV2, MemoryDreamBatch, Goal, Laziness, ImageDescribe, PermissionClassifier] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSnapshot {
    pub origin: RequestOrigin,
    pub observations: Vec<AttemptObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "persistence", rename_all = "snake_case")]
pub enum CaptureReceipt {
    Confirmed { origin: RequestOrigin },
    Unconfirmed { cause: CaptureFailure },
}

impl CaptureReceipt {
    pub fn origin(&self) -> Option<&RequestOrigin> {
        match self {
            Self::Confirmed { origin } => Some(origin),
            Self::Unconfirmed { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestSnapshotNotification {
    #[serde(flatten)]
    pub snapshot: RequestSnapshotRef,
    pub evidence: AttemptObservation,
    pub receipt: CaptureReceipt,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationDurability { Synced, Unconfirmed }

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum PublicationProgress {
    #[default]
    Unknown,
    NotSubmitted,
    NotCommitted,
    JournalPrefix { committed: usize, durability: PublicationDurability },
    AmbiguousSuffix { committed: usize },
    ReplyLost,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PublicationFailure {
    pub cause: CaptureFailure,
    pub progress: PublicationProgress,
}

impl PublicationFailure {
    pub(crate) fn not_submitted(cause: CaptureFailure) -> Self {
        Self { cause, progress: PublicationProgress::NotSubmitted }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePublicationFailure {
    #[serde(default)]
    pub progress: PublicationProgress,
    pub session_id: String,
    pub sources: Vec<SourceOccurrence>,
    pub cause: CaptureFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectedBodyRef {
    pub snapshot: RequestSnapshotRef,
    pub attempt: RequestAttemptId,
    pub digest: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RequestBodyRead {
    Available {
        json: String,
        representation: xai_grok_sampler::request_capture::PayloadRepresentation,
        protected_local: ProtectedBodyRef,
    },
    Unavailable { availability: BodyAvailability },
}

impl std::fmt::Debug for RequestBodyRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Available { representation, protected_local, .. } => f.debug_struct("Available")
                .field("representation", representation).field("protected_local", protected_local).finish_non_exhaustive(),
            Self::Unavailable { availability } => f.debug_struct("Unavailable").field("availability", availability).finish(),
        }
    }
}

pub(crate) enum SourcePublications {
    RestorePending { known: HashSet<SourceOccurrenceId> },
    Ready { known: HashSet<SourceOccurrenceId> },
    Unavailable { known: HashSet<SourceOccurrenceId>, cause: CaptureFailure, notice: FailureNotice },
}

pub(crate) enum FailureNotice { Pending, Sent }

impl Default for SourcePublications {
    fn default() -> Self { Self::Ready { known: HashSet::new() } }
}

impl SourcePublications {
    pub(crate) fn restored() -> Self {
        Self::RestorePending { known: HashSet::new() }
    }

    pub(crate) fn needs_restore(&self) -> bool {
        match self {
            Self::RestorePending { .. } => true,
            Self::Ready { .. } => false,
            Self::Unavailable { cause, .. } => match cause {
                CaptureFailure::InvalidEvidence => false,
                CaptureFailure::Storage | CaptureFailure::Io { .. } | CaptureFailure::ReplyLost
                | CaptureFailure::MailboxClosed | CaptureFailure::OriginUnavailable | CaptureFailure::WorkerLost => true,
            },
        }
    }

    pub(crate) fn restore(&mut self, result: Result<Vec<SourceOccurrenceId>, CaptureFailure>) {
        match result {
            Ok(ids) => {
                let known = match self {
                    Self::RestorePending { known } | Self::Ready { known } | Self::Unavailable { known, .. } => known,
                };
                known.extend(ids);
                *self = Self::Ready { known: std::mem::take(known) };
            }
            Err(cause) => match self {
                Self::Ready { .. } => {}
                Self::RestorePending { known } => {
                    *self = Self::Unavailable { known: std::mem::take(known), cause, notice: FailureNotice::Pending };
                }
                Self::Unavailable { cause: previous, notice, .. } => {
                    if *previous != cause { *notice = FailureNotice::Pending; }
                    *previous = cause;
                }
            },
        }
    }

    pub(crate) fn failure(&self) -> Option<CaptureFailure> {
        match self {
            Self::Ready { .. } => None,
            Self::RestorePending { .. } => Some(CaptureFailure::OriginUnavailable),
            Self::Unavailable { cause, .. } => Some(*cause),
        }
    }

    pub(crate) fn take_failure_notice(&mut self) -> Option<CaptureFailure> {
        match self {
            Self::Unavailable { cause, notice, .. } => {
                match notice {
                    FailureNotice::Pending => { *notice = FailureNotice::Sent; Some(*cause) }
                    FailureNotice::Sent => None,
                }
            }
            Self::Ready { .. } | Self::RestorePending { .. } => None,
        }
    }

    pub(crate) fn settle_failure(&mut self, sources: &[SourceOccurrence], progress: PublicationProgress) {
        let known = match self {
            Self::Ready { known } | Self::RestorePending { known } | Self::Unavailable { known, .. } => known,
        };
        let retained = match progress {
            PublicationProgress::JournalPrefix { committed, .. } | PublicationProgress::AmbiguousSuffix { committed } => committed,
            PublicationProgress::Unknown | PublicationProgress::NotSubmitted | PublicationProgress::NotCommitted | PublicationProgress::ReplyLost => 0,
        };
        for source in sources.iter().skip(retained) {
            if let Some(id) = source.id() { known.remove(&id); }
        }
        match progress {
            PublicationProgress::Unknown | PublicationProgress::ReplyLost | PublicationProgress::AmbiguousSuffix { .. }
            | PublicationProgress::JournalPrefix { committed: 1.., .. } => {
                *self = Self::RestorePending { known: std::mem::take(known) };
            }
            PublicationProgress::NotSubmitted | PublicationProgress::NotCommitted
            | PublicationProgress::JournalPrefix { committed: 0, .. } => {}
        }
    }

    pub(crate) fn claim(&mut self, source: SourceOccurrence) -> bool {
        match self {
            Self::Ready { known } => match source {
                SourceOccurrence::Admitted { id } => known.insert(id),
                SourceOccurrence::LegacyNormalized { .. } | SourceOccurrence::Unadmitted => false,
            },
            Self::RestorePending { .. } | Self::Unavailable { .. } => false,
        }
    }
}

impl std::fmt::Debug for PromptInputRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("PromptInputRef(<opaque>)") }
}

impl std::fmt::Debug for AppliedSourceMeta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("AppliedSourceMeta(<opaque>)") }
}

impl std::fmt::Debug for InputSourceBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("InputSourceBinding(<opaque>)") }
}

impl std::fmt::Debug for InputSourceNotification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("InputSourceNotification(<opaque>)") }
}

impl std::fmt::Debug for SourcePublication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("SourcePublication(<opaque>)") }
}
