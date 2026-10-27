//! Shared domain types for the chat state actor.

use std::collections::BTreeSet;
use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};
use xai_grok_sampling_types::{ConversationItem, ConversationRequest, SamplingConfig};

/// An applied append, without a durability or provider-dispatch claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedAppend {
    pub source: xai_grok_sampling_types::SourceOccurrence,
}

#[derive(Debug)]
pub enum MutationReply {
    Unobserved,
    Observed(tokio::sync::oneshot::Sender<AppliedAppend>),
}

impl MutationReply {
    pub(crate) fn send(self, applied: AppliedAppend) {
        match self {
            Self::Unobserved => {}
            Self::Observed(reply) => { let _ = reply.send(applied); }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendBinding {
    Applied(AppliedAppend),
    ReceiptSourceUnavailable,
    NotApplied,
}

#[derive(Debug, Clone)]
pub struct MutationAck<A> {
    pub acknowledgement: A,
    pub binding: AppendBinding,
}

#[derive(Debug)]
pub enum MutationAdmissionError {
    MailboxClosed(crate::handle::ChatStateMailboxClosed),
    /// The mutation may already have applied.
    ReplyLost(tokio::sync::oneshot::error::RecvError),
}

impl std::fmt::Display for MutationAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MailboxClosed(source) => write!(f, "{source}"),
            Self::ReplyLost(source) => write!(f, "chat-state mutation reply lost: {source}"),
        }
    }
}

impl std::error::Error for MutationAdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MailboxClosed(source) => Some(source),
            Self::ReplyLost(source) => Some(source),
        }
    }
}

/// A policy command was not admitted, or its admitted outcome could not be observed.
#[derive(Debug)]
pub enum ContextPolicyAdmissionError {
    MailboxClosed(crate::handle::ChatStateMailboxClosed),
    /// The command was admitted; a mutation may already have applied.
    ReplyLost(tokio::sync::oneshot::error::RecvError),
}

impl std::fmt::Display for ContextPolicyAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MailboxClosed(source) => write!(f, "{source}"),
            Self::ReplyLost(source) => write!(f, "chat-state policy reply lost: {source}"),
        }
    }
}

impl std::error::Error for ContextPolicyAdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MailboxClosed(source) => Some(source),
            Self::ReplyLost(source) => Some(source),
        }
    }
}

/// Token count relative to the current sampling config / tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenCount {
    /// Last provider-reported total for the current model/window.
    Fresh { tokens: u64 },
    /// Last known total from a previous model/window; keep the number, do not
    /// treat it as a measurement of the current config until the next usage.
    Stale { tokens: u64 },
}

impl Serialize for TokenCount {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.tokens().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TokenCount {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u64::deserialize(deserializer).map(|tokens| Self::Fresh { tokens })
    }
}

impl TokenCount {
    pub const fn tokens(self) -> u64 {
        match self {
            Self::Fresh { tokens } | Self::Stale { tokens } => tokens,
        }
    }

    pub const fn is_stale(self) -> bool {
        matches!(self, Self::Stale { .. })
    }
}

impl Default for TokenCount {
    fn default() -> Self {
        Self::Fresh { tokens: 0 }
    }
}

/// Canonical marker for an injected memory-context block. Shared by emitter and upsert/detection.
/// A drift would silently break dedup and let blocks accumulate in the prompt prefix.
/// Assumes the literal never appears in a system prompt except as an injected block.
pub const MEMORY_CONTEXT_OPEN_TAG: &str = "<memory-context>";

/// Closing tag paired with [`MEMORY_CONTEXT_OPEN_TAG`].
pub const MEMORY_CONTEXT_CLOSE_TAG: &str = "</memory-context>";

/// Lossless record of content that entered a model request view without being persisted as
/// conversation history. The shell relays each record to the CLI, which renders it as a
/// collapsed injected entry — nothing enters the request invisibly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum InjectedRequestRecord {
    /// The memory reminder spliced into this request's system prompt (the non-persist branch of
    /// request assembly: the model sees it, chat history does not).
    MemoryReminder {
        /// Complete reminder body as spliced.
        text: String,
        #[serde(default, skip_serializing_if = "xai_grok_sampling_types::SourceOccurrence::is_unadmitted")]
        source: xai_grok_sampling_types::SourceOccurrence,
    },
    /// Per-turn transforms applied to the stored history while building the request view
    /// (pruning, memory-context withholding, context-policy projection, agent-message labeling).
    ContextView {
        /// Transform summary followed by the resulting request view.
        text: String,
    },
    Synthetic {
        reason: xai_grok_sampling_types::SyntheticReason,
        text: String,
        #[serde(default, skip_serializing_if = "xai_grok_sampling_types::SourceOccurrence::is_unadmitted")]
        source: xai_grok_sampling_types::SourceOccurrence,
    },
}

impl InjectedRequestRecord {
    /// Preserve the complete body; presentation folding must not discard request content.
    pub fn new(kind: InjectedRequestKind, text: &str) -> Self {
        let text = text.to_string();
        match kind {
            InjectedRequestKind::MemoryReminder => Self::MemoryReminder { text, source: Default::default() },
            InjectedRequestKind::ContextView => Self::ContextView { text },
        }
    }

    /// The complete body this record carries.
    pub fn text(&self) -> &str {
        match self {
            Self::MemoryReminder { text, .. } | Self::ContextView { text } | Self::Synthetic { text, .. } => text,
        }
    }

    /// Existing notification kind, resolved from the typed request surface or synthetic origin.
    pub fn kind_wire(&self) -> &'static str {
        match self {
            Self::MemoryReminder { .. } => "memory_reminder",
            Self::ContextView { .. } => "context_view",
            Self::Synthetic { reason, .. } => reason.wire_name(),
        }
    }
}

/// Which request-only surface a record describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectedRequestKind {
    MemoryReminder,
    ContextView,
}

#[derive(Debug, Clone, Copy)]
pub enum MemoryReminderPersistence {
    Canonical,
    RequestOnly,
}

#[derive(Clone)]
pub enum RequestPreparation {
    Sampling { memory_reminder: MemoryReminderPersistence },
    Preview,
    PreviewWith {
        history: PreviewHistory,
        policy: xai_grok_sampling_types::ContextPolicy,
        appended: Vec<ConversationItem>,
    },
}

/// Reply of [`crate::ChatStateHandle::build_request`]: the wire-ready request plus the lossless
/// records of request-only content the CLI renders as collapsed injected entries.
#[derive(Debug, Clone)]
pub struct BuiltConversationRequest {
    pub context_policy: xai_grok_sampling_types::ContextPolicy,
    pub request: ConversationRequest,
    pub injected_records: Vec<InjectedRequestRecord>,
}

// The request is the reply's primary payload; Deref keeps existing field access
// (`built.items`, `built.model`, …) working at the call sites and tests.
impl std::ops::Deref for BuiltConversationRequest {
    type Target = ConversationRequest;

    fn deref(&self) -> &Self::Target {
        &self.request
    }
}

impl std::ops::DerefMut for BuiltConversationRequest {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.request
    }
}



/// Configuration for the ChatStateActor at spawn time.
#[derive(Debug, Clone)]
pub struct ChatStateConfig {
    /// Initial conversation items to populate the state with.
    pub initial_conversation: Vec<ConversationItem>,
    /// Sampling configuration (model, context window, etc.).
    pub sampling_config: SamplingConfig,
}

/// Immutable snapshot of the actor's state (for forking, rewind).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStateSnapshot {
    /// The full conversation history.
    pub conversation: Vec<ConversationItem>,
    /// Current sampling configuration.
    pub sampling_config: SamplingConfig,
    /// Current prompt index (incremented per user turn).
    pub prompt_index: usize,
        /// Accumulated token usage.
    pub total_tokens: TokenCount,
    /// Bytes/4 estimate of the conversation as of the last `record_token_usage`.
    /// `0` means unknown (pre-field snapshot); restore re-estimates instead.
    #[serde(default)]
    pub estimate_at_last_response: u64,
    /// File paths the agent has edited.
    pub agent_edited_paths: BTreeSet<String>,
    /// Cached prompt texts for rewind preview.
    pub prompt_texts: Vec<String>,
    /// Timestamp when the current stream started (epoch ms).
    pub stream_start_ms: Option<i64>,
    /// Timestamp when the current turn started (epoch ms).
    pub turn_start_ms: Option<i64>,
    /// Prompt index at which the last compaction occurred.
    pub last_compaction_prompt_index: Option<usize>,
    /// Opaque credential secrets (API key, optional extra auth, client version).
    #[serde(default)]
    pub credentials: Credentials,
}

/// Metadata for session notifications (timing info).
#[derive(Debug, Clone)]
pub struct NotificationMeta {
    /// Timestamp when the current stream started (epoch ms).
    pub stream_start_ms: Option<i64>,
    /// Timestamp when the current turn started (epoch ms).
    pub turn_start_ms: Option<i64>,
}

/// Configuration for tool-result pruning.
/// Soft trim keeps head + tail; hard clear replaces entirely.
#[derive(Debug, Clone)]
pub struct PruningConfig {
    /// Whether pruning is enabled.
    pub enabled: bool,
    /// Number of recent turns whose tool results are never pruned.
    pub keep_last_n_turns: usize,
    /// Character threshold above which old tool results are soft-trimmed.
    pub soft_trim_threshold: usize,
    /// Characters to keep from the start of a soft-trimmed result.
    pub soft_trim_head: usize,
    /// Characters to keep from the end of a soft-trimmed result.
    pub soft_trim_tail: usize,
    /// Turn age after which tool results are hard-cleared (replaced with placeholder).
    pub hard_clear_age_turns: usize,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_last_n_turns: 3,
            soft_trim_threshold: 4000,
            soft_trim_head: 1500,
            soft_trim_tail: 1500,
            hard_clear_age_turns: 10,
        }
    }
}

/// Where the session's current api_key came from.
/// Determines whether the key can be refreshed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    /// From AuthManager (grok login, OIDC, external binary). Refreshable.
    #[default]
    SessionToken,
    /// From user config ([model.*] api_key, env_key, XAI_API_KEY). Not refreshable.
    ApiKey,
}

/// Credential/secret fields that the actor stores opaquely.
/// Not part of secret-free `SamplingConfig`. The actor stores and returns them — it never interprets them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    /// API key for authentication.
    pub api_key: Option<String>,
    /// Whether this is a session token (refreshable) or user-provided api key.
    #[serde(default)]
    pub auth_type: AuthType,
    /// Optional extra auth material forwarded with requests when present.
    pub alpha_test_key: Option<String>,
    /// Client version string.
    pub client_version: Option<String>,
}

/// The messages captured during a single conversation turn.
///
/// Produced by `TakeTurnMessages` after a `BeginTurnCapture`/message-push cycle.
#[derive(Debug, Clone)]
pub struct TurnCapture {
    /// The ordered sequence of messages appended during this turn.
    pub messages: Vec<ConversationItem>,
    /// Whether compaction (conversation replacement) occurred mid-turn.
    pub compaction_occurred: bool,
}

/// Item counts for a conversation, broken down by role.
/// Avoids cloning the conversation when only role counts and total length are needed.
#[derive(Debug, Clone, Default)]
pub struct ConversationCounts {
    /// Total number of items in the conversation.
    pub total: usize,
    /// Number of `User` items.
    pub user: usize,
    /// Number of `Assistant` items.
    pub assistant: usize,
    /// Number of `ToolResult` items.
    pub tool_result: usize,
}

/// Info returned when auto-compact threshold is exceeded.
#[derive(Debug, Clone)]
pub struct AutoCompactTrigger {
    /// Current total token count.
    pub total_tokens: u64,
    /// Model's context window size.
    pub context_window: NonZeroU64,
    /// Current utilization as a percentage (0–100).
    pub utilization_percent: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trips_through_serde_json() {
        let snapshot = ChatStateSnapshot {
            conversation: vec![],
            sampling_config: SamplingConfig {
                base_url: "https://api.example.com".to_string(),
                model: "test-model".to_string(),
                max_retries: Some(6),
                rate_limit_retry_threshold: Some(4),
                context_window: NonZeroU64::new(128_000).unwrap(),
                ..Default::default()
            },
            prompt_index: 0,
            total_tokens: TokenCount::Fresh { tokens: 0 },
            estimate_at_last_response: 0,
            agent_edited_paths: BTreeSet::new(),
            prompt_texts: vec![],
            stream_start_ms: None,
            turn_start_ms: None,
            last_compaction_prompt_index: None,
            credentials: Credentials::default(),
        };

        let json = serde_json::to_string(&snapshot).expect("serialize");
        let deserialized: ChatStateSnapshot = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.prompt_index, 0);
        assert_eq!(deserialized.total_tokens.tokens(), 0);
        assert!(deserialized.conversation.is_empty());
        assert!(deserialized.agent_edited_paths.is_empty());
        assert!(deserialized.last_compaction_prompt_index.is_none());
        assert_eq!(deserialized.sampling_config.max_retries, Some(6));
        assert_eq!(
            deserialized.sampling_config.rate_limit_retry_threshold,
            Some(4)
        );
    }

    #[test]
    fn snapshot_round_trips_with_data() {
        use xai_grok_sampling_types::ConversationItem;

        let snapshot = ChatStateSnapshot {
            conversation: vec![
                ConversationItem::system("You are a helpful assistant."),
                ConversationItem::user("Hello!"),
                ConversationItem::assistant("Hi there!"),
            ],
            sampling_config: SamplingConfig {
                base_url: "https://api.example.com".to_string(),
                model: "grok-3".to_string(),
                max_completion_tokens: Some(4096),
                temperature: Some(0.7),
                context_window: NonZeroU64::new(128_000).unwrap(),
                ..Default::default()
            },
            prompt_index: 5,
            total_tokens: TokenCount::Fresh { tokens: 1234 },
            estimate_at_last_response: 900,
            agent_edited_paths: BTreeSet::from([
                "src/main.rs".to_string(),
                "src/lib.rs".to_string(),
            ]),
            prompt_texts: vec!["first prompt".to_string(), "second prompt".to_string()],
            stream_start_ms: Some(1234567890),
            turn_start_ms: Some(1234567800),
            last_compaction_prompt_index: Some(2),
            credentials: Credentials::default(),
        };

        let json = serde_json::to_string(&snapshot).expect("serialize");
        let deserialized: ChatStateSnapshot = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.prompt_index, 5);
        assert_eq!(deserialized.total_tokens.tokens(), 1234);
        assert_eq!(deserialized.conversation.len(), 3);
        assert_eq!(deserialized.agent_edited_paths.len(), 2);
        assert_eq!(deserialized.prompt_texts.len(), 2);
        assert_eq!(deserialized.stream_start_ms, Some(1234567890));
        assert_eq!(deserialized.turn_start_ms, Some(1234567800));
        assert_eq!(deserialized.last_compaction_prompt_index, Some(2));
    }
}


pub struct ChatStatePreview {
    pub canonical_history: Vec<ConversationItem>,
    pub sampling_config: xai_grok_sampling_types::SamplingConfig,
    pub built: BuiltConversationRequest,
}

impl std::fmt::Debug for ChatStatePreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatStatePreview").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewFailure { MailboxClosed, ReplyLost }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewHistory { Stable, InFlight }

impl std::fmt::Debug for RequestPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sampling { memory_reminder } => f.debug_tuple("Sampling").field(memory_reminder).finish(),
            Self::Preview => f.write_str("Preview"),
            Self::PreviewWith { .. } => f.write_str("PreviewWith { .. }"),
        }
    }
}
