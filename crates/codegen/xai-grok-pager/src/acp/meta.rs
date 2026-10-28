//! Strongly-typed notification metadata.
//!
//! Parses the `_meta` JSON from `SessionNotification` into a struct with typed fields.
//! Missing legacy provenance stays uncorrelated; malformed sampling provenance remains distinct.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};
use xai_grok_shell::extensions::sampling_lifecycle::{
    SAMPLING_LIFECYCLE_META_KEY, SAMPLING_OWNER_META_KEY, SamplingLifecycle, SamplingOwner,
};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub enum SamplingMetadata {
    #[default]
    Legacy,
    Malformed,
    Owner(SamplingOwner),
    Lifecycle(SamplingLifecycle),
}

impl SamplingMetadata {
    fn owner(&self) -> Option<&SamplingOwner> {
        match self {
            Self::Owner(owner) | Self::Lifecycle(SamplingLifecycle { owner, .. }) => Some(owner),
            Self::Legacy | Self::Malformed => None,
        }
    }

    fn parse(meta: &acp::Meta, explicit: Option<&SamplingLifecycle>) -> Self {
        let parse = || -> Result<Self, ()> {
            let owner = meta
                .get(SAMPLING_OWNER_META_KEY)
                .map(|value| serde_json::from_value::<SamplingOwner>(value.clone()).map_err(|_| ()))
                .transpose()?;
            let lifecycle = meta
                .get(SAMPLING_LIFECYCLE_META_KEY)
                .map(|value| {
                    serde_json::from_value::<SamplingLifecycle>(value.clone()).map_err(|_| ())
                })
                .transpose()?;
            let lifecycle = match (lifecycle, explicit) {
                (Some(stamped), Some(explicit)) if &stamped != explicit => return Err(()),
                (Some(stamped), _) => Some(stamped),
                (None, Some(explicit)) => Some(explicit.clone()),
                (None, None) => None,
            };
            let owner = match (owner, lifecycle.as_ref()) {
                (Some(owner), Some(lifecycle)) if owner != lifecycle.owner => return Err(()),
                (Some(owner), _) => owner,
                (None, Some(lifecycle)) => lifecycle.owner.clone(),
                (None, None) => return Ok(Self::Legacy),
            };
            if owner.prompt_id.is_empty()
                || owner.request_id.as_str().is_empty()
                || owner.attempt.0 == 0
            {
                return Err(());
            }
            if meta
                .get("promptId")
                .is_some_and(|value| value.as_str() != Some(owner.prompt_id.as_str()))
                || meta
                    .get("requestId")
                    .is_some_and(|value| value.as_str() != Some(owner.request_id.as_str()))
                || meta
                    .get("streamEpoch")
                    .is_some_and(|value| value.as_u64() != Some(owner.attempt.0))
            {
                return Err(());
            }
            Ok(match lifecycle {
                Some(lifecycle) => Self::Lifecycle(lifecycle),
                None => Self::Owner(owner),
            })
        };
        match parse() {
            Ok(parsed) => parsed,
            Err(()) => Self::Malformed,
        }
    }
}

/// Parsed fields from `SessionNotification._meta`.
///
/// Extracted once in [`acp_handler`](super::super::app::acp_handler) and passed downstream to the tracker and agent state.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub(crate) enum CurrentSourceMetadata {
    #[default]
    Legacy,
    Malformed,
    Version(xai_grok_shell::session::current_actions::SourceVersion),
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NotificationMeta {
    /// Accumulated token count across the session (`totalTokens`).
    pub total_tokens: Option<u64>,
    /// UTC ms when this notification was sent (`agentTimestampMs`).
    pub agent_timestamp_ms: Option<i64>,
    /// UTC ms when the current LLM streaming response started (`streamStartMs`).
    /// Resets each tool-use loop iteration.
    pub stream_start_ms: Option<i64>,
    /// UTC ms when the current turn started (`turnStartMs`).
    /// Constant for the entire turn.
    pub turn_start_ms: Option<i64>,
    /// Stable id for the prompt this notification belongs to (`promptId`).
    /// The client passes a UUID in `PromptRequest._meta.promptId`; the agent echoes it on every notification it emits while processing that prompt.
    /// Used to drop chunks for cancelled / rewound turns.
    pub prompt_id: Option<String>,
    /// Whether this notification is historical replay from `session/load`.
    pub is_replay: bool,
    /// Raw `eventId` string (`"{sessionId}-{counter}"`). The agent resolves it by exact string match against persisted
    /// lines, so the full id is kept.
    pub event_id: Option<String>,
    /// Monotonic per-process sequence parsed from `eventId` (`"{sessionId}-{counter}"`, see `xai-grok-shell
    /// util::event_id`). Per-session events arrive in increasing order, so the pager keeps a highwater and drops
    /// anything at or below it. such updates always apply.
    pub event_seq: Option<u64>,
    /// Sampler request that produced this buffered xAI delta (`requestId`). Absent on old shells.
    pub xai_request_id: Option<String>,
    /// Retry-attempt epoch of that request (`streamEpoch`); changes when the scheduler discards
    /// an attempt. Absent on old shells.
    pub xai_stream_epoch: Option<u64>,
    #[serde(default)]
    pub sampling: SamplingMetadata,
    #[serde(default)]
    pub(crate) current_actions_version: CurrentSourceMetadata,
    #[serde(default)]
    pub(crate) delivery_role: xai_grok_shell::extensions::current_actions::DeliveryRole,
    #[serde(skip)]
    pub(crate) tool: super::tool_meta::ToolMetadata,
}

pub(crate) fn tool_completion(
    status: acp::ToolCallStatus,
    meta: Option<&acp::Meta>,
) -> crate::scrollback::types::ToolCompletion {
    use crate::scrollback::types::ToolCompletion;
    use xai_grok_shell::session::acp_types::{TOOL_TERMINAL_STATUS_META_KEY, ToolTerminalStatus};
    let terminal = meta
        .and_then(|meta| meta.get(TOOL_TERMINAL_STATUS_META_KEY))
        .and_then(|value| serde_json::from_value::<ToolTerminalStatus>(value.clone()).ok());
    match (status, terminal) {
        (acp::ToolCallStatus::Failed, Some(ToolTerminalStatus::Cancelled)) => {
            ToolCompletion::Cancelled
        }
        (status, _) => status.into(),
    }
}

/// Request/attempt provenance of a buffered `ToolCallDeltaChunk`, parsed from its `_meta`.
/// Equal provenance means the deltas belong to one continuous argument stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WritingDeltaStream {
    Legacy {
        request_id: Option<String>,
        stream_epoch: Option<u64>,
    },
    Owned(SamplingOwner),
}

impl Default for WritingDeltaStream {
    fn default() -> Self {
        Self::Legacy {
            request_id: None,
            stream_epoch: None,
        }
    }
}

/// Serializable counterpart of the replay stamp the agent injects on replayed notifications.
/// [`NotificationMeta::from_json`] is the parse side. this is the build side. That spares those sites from
/// hand-writing `json!` literals.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayMetaStamp {
    pub is_replay: bool,
}

impl ReplayMetaStamp {
    /// `_meta` value for a replayed (`session/load`) notification.
    pub fn replayed() -> serde_json::Value {
        serde_json::to_value(Self { is_replay: true }).expect("serialize replay meta stamp")
    }

    /// The replay stamp as an ACP `Meta`.
    pub fn replayed_meta() -> acp::Meta {
        Self::replayed()
            .as_object()
            .cloned()
            .expect("replay meta stamp is an object")
    }
}

/// User-prompt content-block `_meta` keys (`TextContent.meta`). Shared by the producers (`dispatch/queue.rs` drain,
/// `effects.rs` prompt send) and the replay consumer (`acp/tracker.rs` `handle_user_message`). Sharing the
/// constants keeps the wire keys from drifting. Tests keep raw literals; they pin the wire values.
pub mod user_prompt_meta {
    /// Clean display text shown in scrollback instead of the wire text.
    pub const DISPLAY_TEXT: &str = "displayText";
    /// Render the display text as a skill invocation (teal leading token).
    pub const DISPLAY_AS_SKILL: &str = "displayAsSkill";
    /// Render the display text as a scheduled (cron) prompt.
    pub const DISPLAY_AS_CRON: &str = "displayAsCron";
    /// `[[start, end], …]` byte ranges of recognized slash tokens into the block's `text`.
    /// Only meaningful when that text is displayed verbatim (never stamped alongside `displayText`).
    pub const SKILL_TOKEN_RANGES: &str = "skillTokenRanges";
    /// See [`xai_prompt_queue::COMBINED_DISPLAY_TEXTS_META`].
    pub const COMBINED_DISPLAY_TEXTS: &str = xai_prompt_queue::COMBINED_DISPLAY_TEXTS_META;
}

/// `UserMessageChunk` / `ContentChunk._meta` keys stamped by the shell and
/// read by the pager (live and on replay).
pub mod user_message_chunk_meta {
    /// Prompt index for rewind / attribution.
    pub const PROMPT_INDEX: &str = "promptIndex";
    /// When true, the chunk must stay out of scrollback user prompts. Legacy records only:
    /// the shell now stamps [`SYNTHETIC_REASON`] instead, so such chunks render as collapsed
    /// injected entries rather than being dropped.
    pub const HIDE_FROM_SCROLLBACK: &str = "hideFromScrollback";
    /// When true, the chunk is a persisted mid-turn interjection; replay renders its `displayText` as an interjection block.
    pub const INTERJECTION: &str = xai_grok_shell::session::storage::INTERJECTION_META_KEY;
    /// Daemon `UserMessage.message_id`. An interjection stamps this with its injection id.
    pub const MESSAGE_ID: &str = "messageId";
    /// `SyntheticReason` wire name of the persisted item this chunk echoes; the tracker routes
    /// the chunk to a collapsed-by-default injected entry. Shared const with the shell stamper.
    pub const SYNTHETIC_REASON: &str = xai_grok_shell::session::storage::SYNTHETIC_REASON_META_KEY;
}

/// Extract the numeric counter from an `eventId` (`"{sessionId}-{counter}"`).
/// The counter is the part after the last `-` (session ids themselves contain `-`).
pub fn event_id_counter(event_id: &str) -> Option<u64> {
    event_id
        .rsplit('-')
        .next()
        .and_then(|c| c.parse::<u64>().ok())
}

impl NotificationMeta {
    /// Parse from the `_meta` JSON map on a `SessionNotification`.
    pub fn from_json(meta: Option<&serde_json::Map<String, serde_json::Value>>) -> Self {
        Self::parse_json(meta, None)
    }

    fn parse_json(meta: Option<&acp::Meta>, lifecycle: Option<&SamplingLifecycle>) -> Self {
        let empty = acp::Meta::new();
        let m = meta.unwrap_or(&empty);
        let event_id = m
            .get("eventId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let event_seq = event_id.as_deref().and_then(event_id_counter);
        let sampling = SamplingMetadata::parse(m, lifecycle);
        let owner = sampling.owner();
        let delivery_role =
            xai_grok_shell::extensions::current_actions::delivery_role(meta).unwrap_or_default();
        Self {
            total_tokens: m.get("totalTokens").and_then(|v| v.as_u64()),
            agent_timestamp_ms: m.get("agentTimestampMs").and_then(|v| v.as_i64()),
            stream_start_ms: m.get("streamStartMs").and_then(|v| v.as_i64()),
            turn_start_ms: m.get("turnStartMs").and_then(|v| v.as_i64()),
            prompt_id: m
                .get("promptId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| owner.map(|owner| owner.prompt_id.clone())),
            is_replay: match &delivery_role {
                xai_grok_shell::extensions::current_actions::DeliveryRole::Historical {
                    ..
                } => true,
                xai_grok_shell::extensions::current_actions::DeliveryRole::Live { .. }
                | xai_grok_shell::extensions::current_actions::DeliveryRole::LiveUnbound {
                    ..
                } => false,
                xai_grok_shell::extensions::current_actions::DeliveryRole::Unavailable => {
                    m.get("isReplay").and_then(|v| v.as_bool()).unwrap_or(false)
                }
            },
            delivery_role,
            xai_request_id: m
                .get("requestId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            xai_stream_epoch: m.get("streamEpoch").and_then(|v| v.as_u64()),
            event_id,
            event_seq,
            sampling,
            current_actions_version: match m
                .get(xai_grok_shell::session::current_actions::VERSION_META_KEY)
            {
                None => CurrentSourceMetadata::Legacy,
                Some(value) => match serde_json::from_value(value.clone()) {
                    Ok(version) => CurrentSourceMetadata::Version(version),
                    Err(_) => CurrentSourceMetadata::Malformed,
                },
            },
            tool: if ["promptId", "requestId"].iter().any(|key| {
                m.get(*key)
                    .is_some_and(|value| value.as_str().is_none_or(str::is_empty))
            }) || m
                .get("streamEpoch")
                .is_some_and(|value| value.as_u64().is_none())
                || m.get("isReplay").is_some_and(|value| !value.is_boolean())
                || m.get("eventId")
                    .is_some_and(|value| value.as_str().is_none_or(str::is_empty))
            {
                super::tool_meta::ToolMetadata::Malformed
            } else {
                super::tool_meta::ToolMetadata::NotTool
            },
        }
    }

    pub(crate) fn from_xai(
        raw: Option<&serde_json::Value>,
        update: &xai_grok_shell::extensions::notification::SessionUpdate,
    ) -> Self {
        use xai_grok_shell::extensions::notification::SessionUpdate;
        if raw.is_some_and(|raw| !raw.is_object()) {
            return Self {
                sampling: SamplingMetadata::Malformed,
                ..Self::default()
            };
        }
        let lifecycle = if let SessionUpdate::SamplingLifecycle { lifecycle } = update {
            Some(lifecycle)
        } else {
            None
        };
        let mut meta = Self::parse_json(raw.and_then(serde_json::Value::as_object), lifecycle);
        if !matches!(
            update,
            SessionUpdate::RetryState(_) | SessionUpdate::SamplingLifecycle { .. }
        ) && matches!(&meta.sampling, SamplingMetadata::Lifecycle(_))
        {
            meta.sampling = SamplingMetadata::Malformed;
        }
        meta
    }

    pub(crate) fn with_tool(mut self, update: &acp::SessionUpdate) -> Self {
        if !matches!(self.tool, super::tool_meta::ToolMetadata::Malformed) {
            self.tool = super::tool_meta::ToolMetadata::parse(update, &self);
        }
        self
    }

    pub(crate) fn sampling_owner(&self) -> Option<&SamplingOwner> {
        self.sampling.owner()
    }

    pub(crate) fn writing_stream(&self) -> WritingDeltaStream {
        match self.sampling_owner() {
            Some(owner) => WritingDeltaStream::Owned(owner.clone()),
            None => WritingDeltaStream::Legacy {
                request_id: self.xai_request_id.clone(),
                stream_epoch: self.xai_stream_epoch,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_full_meta() {
        let meta_json = json!({
            "totalTokens": 5000u64,
            "agentTimestampMs": 1700000000000i64,
            "streamStartMs": 1700000000000i64 - 3200,
            "turnStartMs": 1700000000000i64 - 5000,
            "eventId": "sess-1-7",
        });
        let map = meta_json.as_object().unwrap();
        let meta = NotificationMeta::from_json(Some(map));

        assert_eq!(meta.total_tokens, Some(5000));
        assert_eq!(meta.agent_timestamp_ms, Some(1700000000000));
        assert_eq!(meta.stream_start_ms, Some(1700000000000 - 3200));
        assert_eq!(meta.turn_start_ms, Some(1700000000000 - 5000));
        assert!(!meta.is_replay);
        assert_eq!(meta.event_id.as_deref(), Some("sess-1-7"));
        assert_eq!(meta.event_seq, Some(7));
    }

    #[test]
    fn parse_missing_new_fields() {
        // Simulate old grok-shell that doesn't send streamStartMs/turnStartMs
        let meta_json = json!({
            "totalTokens": 1000u64,
            "agentTimestampMs": 1700000000000i64,
        });
        let map = meta_json.as_object().unwrap();
        let meta = NotificationMeta::from_json(Some(map));

        assert_eq!(meta.total_tokens, Some(1000));
        assert_eq!(meta.agent_timestamp_ms, Some(1700000000000));
        assert_eq!(meta.stream_start_ms, None);
        assert_eq!(meta.turn_start_ms, None);
        assert!(!meta.is_replay);
    }

    #[test]
    fn parse_replay_flag() {
        let meta_json = json!({
            "isReplay": true,
        });
        let map = meta_json.as_object().unwrap();
        let meta = NotificationMeta::from_json(Some(map));

        assert!(meta.is_replay);
    }

    /// The build side ([`ReplayMetaStamp::replayed`]) and the parse side ([`NotificationMeta::from_json`]) must agree on the wire key.
    /// A rename on either side breaks replay detection silently.
    #[test]
    fn replay_meta_stamp_round_trips_through_parser() {
        let stamp = ReplayMetaStamp::replayed();
        let map = stamp.as_object().unwrap();
        let meta = NotificationMeta::from_json(Some(map));
        assert!(meta.is_replay, "stamped meta must parse as a replay");
    }

    #[test]
    fn parse_event_id_keeps_raw_string_and_seq() {
        let meta_json = json!({
            "eventId": "sess-ab-12-42",
        });
        let map = meta_json.as_object().unwrap();
        let meta = NotificationMeta::from_json(Some(map));

        assert_eq!(meta.event_id.as_deref(), Some("sess-ab-12-42"));
        assert_eq!(meta.event_seq, Some(42));
    }

    /// A non-numeric suffix yields no seq (dedup disabled) but the raw id is still kept for the reconnect cursor (string-matched agent-side).
    #[test]
    fn parse_event_id_without_numeric_suffix() {
        let meta_json = json!({
            "eventId": "weird-id-zzz",
        });
        let map = meta_json.as_object().unwrap();
        let meta = NotificationMeta::from_json(Some(map));

        assert_eq!(meta.event_id.as_deref(), Some("weird-id-zzz"));
        assert_eq!(meta.event_seq, None);
    }

    #[test]
    fn parse_none_meta() {
        let meta = NotificationMeta::from_json(None);
        assert_eq!(meta.total_tokens, None);
        assert_eq!(meta.agent_timestamp_ms, None);
        assert_eq!(meta.stream_start_ms, None);
        assert_eq!(meta.turn_start_ms, None);
        assert!(!meta.is_replay);
        assert_eq!(meta.event_id, None);
        assert_eq!(meta.event_seq, None);
        assert_eq!(meta.xai_request_id, None);
        assert_eq!(meta.xai_stream_epoch, None);
    }

    #[test]
    fn parse_xai_delta_provenance() {
        let meta_json = json!({
            "promptId": "p-1",
            "requestId": "req-9",
            "streamEpoch": 2u64,
        });
        let map = meta_json.as_object().unwrap();
        let meta = NotificationMeta::from_json(Some(map));

        assert_eq!(meta.xai_request_id.as_deref(), Some("req-9"));
        assert_eq!(meta.xai_stream_epoch, Some(2));
    }
}
