//! Session transcript replay: line peeks, rewind-aware prepare, and bounded streaming load.
//!
//! Invariants:
//! - InProgress `tool_call_update` peeks are typed serde (unknown fields ignored) so `content` / `rawOutput` are not allocated.
//! - Production child/fork replay streams one typed ACP update at a time ([`stream_replay_updates_at`]).
//!   [`load_updates_for_replay_at`] stays a typed materialize-all reference for tests.

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};

use agent_client_protocol as acp;

use super::relocation::RelocationView;
use super::{
    RawLinePeek, RawParamsPeek, RawUpdatePeek, SessionUpdate, SessionUpdateEnvelope,
    filter_rewind_lines, replay_updates_path_in_dir, strip_context_wrappers,
};
use crate::extensions::notification::SessionNotification;
use crate::extensions::notification::SessionUpdate as XaiUpdate;
use crate::session::wire_tags::{
    AVAILABLE_COMMANDS_UPDATE, TOOL_CALL_STATUS_IN_PROGRESS, TOOL_CALL_UPDATE,
};

/// `_meta` key holding the running token count.
/// The serde `rename` below must match it by hand (serde attrs can't reference a const).
const TOTAL_TOKENS_KEY: &str = "totalTokens";
/// `_meta` key holding the per-event id used for cursor-based reconnect.
const EVENT_ID_KEY: &str = "eventId";

/// What to do when cwd hints miss `updates.jsonl`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReplayLookupFallback {
    #[default]
    Relocation,
    HintedOnly,
}

/// Optional location hints so child `updates.jsonl` lookup can skip a full
/// `~/.grok/sessions` RelocationView scan.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplayPathHint<'a> {
    /// Parent session working directory; tried as `<sessions>/<encoded_cwd>/<child_id>/updates.jsonl`.
    pub parent_cwd: Option<&'a Path>,
    /// Child working directory when it differs from the parent (worktree / custom cwd).
    /// Tried before [`Self::parent_cwd`].
    pub child_cwd: Option<&'a Path>,
    /// When cwd hints miss: scan via [`RelocationView`], or return None.
    pub fallback: ReplayLookupFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnfinishedSubagent {
    pub(crate) subagent_id: String,
    pub(crate) attempt_id: Option<String>,
    pub(crate) child_session_id: String,
}

#[doc(hidden)]
pub struct PreparedReplay<'a> {
    /// Rewind-filtered replay lines, each borrowed from the input transcript.
    pub lines: Vec<&'a str>,
    pub(crate) mark_replay: bool,
    pub(crate) last_tokens: u64,
    /// Highest `eventId` counter across all live (rewind-filtered) lines.
    /// It re-seeds the process-global event counter on resume so post-load live events keep monotonically increasing ids.
    /// `None` when no line carried a parseable `eventId` (older shell).
    pub(crate) max_event_seq: Option<u64>,
    pub(crate) total_live: usize,
    /// Replayed spawns with no matching finish (a rewind can drop the finish), reconciled on load.
    pub(crate) unfinished_subagents: Vec<UnfinishedSubagent>,
}

/// Gates the caller's post-replay memory purge (`Empty` means nothing was reclaimable).
/// For child hydrate it also gates whether disk proved it can rebuild the transcript.
/// xAI events alone never count as `Emitted`, so an eviction decision can't settle on a file the client cannot rebuild content from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ReplayEmission {
    Emitted,
    Empty,
}

/// One update forwarded by the streaming child replay.
/// The stream carries ACP transcript updates plus persisted xAI child events (compaction, retry, memory lifecycle) in file order.
/// The file order lets a rebuilt view keep its non-ACP markers.
pub enum ReplayedUpdate {
    /// The second field is the persisted line's `_meta` (original `agentTimestampMs`/`turnStartMs`).
    /// That is how rebuilt entries keep their run-time timestamps.
    /// For a collapsed ToolCall it is the completing line's meta; `None` for start-only tools flushed at EOF.
    Acp(acp::SessionUpdate, Option<acp::Meta>),
    Xai(XaiUpdate, Option<serde_json::Value>),
}

impl std::fmt::Debug for ReplayedUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Acp(_, _) => "ReplayedUpdate::Acp",
            Self::Xai(_, _) => "ReplayedUpdate::Xai",
        })
    }
}

/// Collapses a ToolCall and its ToolCallUpdates into one ToolCall during replay.
/// Parent forward never flushes leftovers (cursor/`_meta.eventId` contract).
/// Child stream EOF calls [`Self::take_pending`] so start-only tools still hydrate.
pub(crate) struct ReplayToolCollapser {
    pending: BTreeMap<std::sync::Arc<str>, acp::ToolCall>,
}

impl ReplayToolCollapser {
    pub(crate) fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
        }
    }

    /// Ingest one ACP update. `None` means hold or drop (do not forward yet).
    pub(crate) fn push(&mut self, update: acp::SessionUpdate) -> Option<acp::SessionUpdate> {
        match update {
            acp::SessionUpdate::AvailableCommandsUpdate(_) => None,
            acp::SessionUpdate::ToolCall(tc) => {
                if matches!(
                    tc.status,
                    acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed
                ) {
                    return Some(acp::SessionUpdate::ToolCall(tc));
                }
                self.pending.insert(tc.tool_call_id.0.clone(), tc);
                None
            }
            acp::SessionUpdate::ToolCallUpdate(mut u) => match u.fields.status {
                Some(acp::ToolCallStatus::Completed) | Some(acp::ToolCallStatus::Failed) => {
                    if let Some(mut base) = self.pending.remove(&u.tool_call_id.0) {
                        base.update(std::mem::take(&mut u.fields));
                        return Some(acp::SessionUpdate::ToolCall(base));
                    }
                    Some(acp::SessionUpdate::ToolCallUpdate(u))
                }
                None => {
                    if let Some(base) = self.pending.get_mut(&u.tool_call_id.0) {
                        base.update(std::mem::take(&mut u.fields));
                    }
                    None
                }
                Some(_) => None,
            },
            other => Some(other),
        }
    }

    /// Child-stream EOF: emit ToolCalls that never saw Completed/Failed.
    pub(crate) fn take_pending(&mut self) -> impl Iterator<Item = acp::SessionUpdate> {
        std::mem::take(&mut self.pending)
            .into_values()
            .map(acp::SessionUpdate::ToolCall)
    }

    fn pop_pending(&mut self) -> Option<acp::SessionUpdate> {
        self.pending.pop_first().map(|(_, call)| acp::SessionUpdate::ToolCall(call))
    }

    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

/// Load replay-ready typed ACP updates for a session, or `None` when the session or its `updates.jsonl` is missing.
pub fn load_updates_for_replay(
    session_id: &str,
) -> std::io::Result<Option<Vec<acp::SessionUpdate>>> {
    let Some(session_dir) =
        crate::session::persistence::find_persisted_session_dir_by_id_result(session_id)?
    else {
        return Ok(None);
    };
    let Some(updates_path) = replay_updates_path_in_dir(&session_dir) else {
        return Ok(None);
    };
    Ok(Some(collect_replay_updates(&updates_path)?))
}

/// Like [`load_updates_for_replay`], but resolves the session under a specific grok home.
/// Typed, materialize-all replay reader: collects every update into owned `Vec`s.
/// Only tests call it: the `testkit_synth_roundtrip` and `session_load_perf` parity references and the in-crate relocation tests.
#[cfg(any(test, feature = "test-support"))]
pub fn load_updates_for_replay_at(
    session_id: &str,
    grok_home: &std::path::Path,
) -> std::io::Result<Option<Vec<acp::SessionUpdate>>> {
    let Some(updates_path) =
        resolve_replay_updates_path(session_id, grok_home, ReplayPathHint::default())?
    else {
        return Ok(None);
    };
    Ok(Some(collect_replay_updates(&updates_path)?))
}

/// This is the materializing counterpart of the streaming [`for_each_replay_update_in_file`].
fn collect_replay_updates(
    updates_path: &std::path::Path,
) -> std::io::Result<Vec<acp::SessionUpdate>> {
    let mut acp_updates: Vec<acp::SessionUpdate> = Vec::new();
    for_each_replay_update_in_file(updates_path, |u| acp_updates.push(u))?;
    Ok(acp_updates)
}

fn is_safe_session_id_component(session_id: &str) -> bool {
    let mut parts = Path::new(session_id).components();
    matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
}

fn try_fast_replay_updates_path(
    session_id: &str,
    grok_home: &Path,
    hint: ReplayPathHint<'_>,
) -> Option<PathBuf> {
    if !is_safe_session_id_component(session_id) {
        return None;
    }
    for cwd in [hint.child_cwd, hint.parent_cwd].into_iter().flatten() {
        let encoded = xai_grok_config::encode_cwd_dirname(&cwd.to_string_lossy());
        let candidate = grok_home.join("sessions").join(encoded).join(session_id);
        if let Some(path) = replay_updates_path_in_dir(&candidate) {
            return Some(path);
        }
    }
    None
}

/// Resolve `updates.jsonl` for `session_id` under `grok_home`, or `None` when the session directory or the file is missing.
/// Shared by the typed `load_updates_for_replay_at` and the streaming [`stream_replay_updates_at`].
pub(crate) fn resolve_replay_updates_path(
    session_id: &str,
    grok_home: &std::path::Path,
    hint: ReplayPathHint<'_>,
) -> std::io::Result<Option<std::path::PathBuf>> {
    match try_fast_replay_updates_path(session_id, grok_home, hint) {
        Some(path) => return Ok(Some(path)),
        None if hint.fallback == ReplayLookupFallback::HintedOnly => {
            // The cwd hints missed; skip the RelocationView scan, which would run on the UI thread
            return Ok(None);
        }
        None => {}
    }
    let sessions_root = grok_home.join("sessions");
    let view = RelocationView::load_for_sessions_root(&sessions_root).map_err(io::Error::other)?;
    let Some(session_dir) = view
        .find_persisted_session_dir(session_id)
        .map_err(io::Error::other)?
    else {
        return Ok(None);
    };
    Ok(replay_updates_path_in_dir(&session_dir))
}

/// Invoke `f` once per client-replay ACP update for a session under `grok_home`, never building the full typed `Vec`.
/// Skips ACU / InProgress lines before full notification serde and collapses a ToolCall with its updates.
/// The typed [`load_updates_for_replay_at`] reference does not skip those.
pub fn stream_replay_updates_at<F: FnMut(acp::SessionUpdate)>(
    session_id: &str,
    grok_home: &std::path::Path,
    mut f: F,
) -> std::io::Result<ReplayEmission> {
    stream_replay_updates_at_hinted(session_id, grok_home, ReplayPathHint::default(), |update| {
        if let ReplayedUpdate::Acp(update, _) = update {
            f(update);
        }
    })
}

/// Whether replaying `session_id`'s persisted transcript would emit at least one ACP update ([`ReplayEmission::Emitted`]), without applying anything.
/// xAI-only, torn, or catalog-only content replays `Empty`.
/// `false` and `Err` both mean "keep the in-memory copy".
pub fn replay_would_emit(
    session_id: &str,
    grok_home: &std::path::Path,
    hint: ReplayPathHint<'_>,
) -> std::io::Result<bool> {
    let Some(path) = resolve_replay_updates_path(session_id, grok_home, hint)? else {
        return Ok(false);
    };
    let raw_contents = std::fs::read_to_string(&path)?;
    for line in rewind_filtered_live(&raw_contents) {
        if line_is_dropped_on_replay(line) {
            continue;
        }
        let Ok(SessionUpdate::Acp(notif)) = SessionUpdateEnvelope::from_str(line) else {
            continue;
        };
        // Mirrors [`ReplayToolCollapser`]
        // Every parsed ACP line reaches the stream (directly, merged into its ToolCall, or via the EOF pending flush)
        // The exceptions are a command catalog and a ToolCallUpdate that never completes and has no base to merge into
        match notif.update {
            acp::SessionUpdate::AvailableCommandsUpdate(_) => {}
            acp::SessionUpdate::ToolCallUpdate(u) => {
                if matches!(
                    u.fields.status,
                    Some(acp::ToolCallStatus::Completed) | Some(acp::ToolCallStatus::Failed)
                ) {
                    return Ok(true);
                }
            }
            _ => return Ok(true),
        }
    }
    Ok(false)
}

/// [`stream_replay_updates_at`] with parent/child cwd hints so child hydrate can skip a full sessions-root scan on the common encoded-cwd path.
/// Persisted xAI child events are forwarded in file order (see [`ReplayedUpdate`]).
pub fn stream_replay_updates_at_hinted<F: FnMut(ReplayedUpdate)>(
    session_id: &str,
    grok_home: &std::path::Path,
    hint: ReplayPathHint<'_>,
    mut f: F,
) -> std::io::Result<ReplayEmission> {
    let Some(updates_path) = resolve_replay_updates_path(session_id, grok_home, hint)? else {
        return Ok(ReplayEmission::Empty);
    };
    let raw_contents = std::fs::read_to_string(&updates_path)?;
    let live = rewind_filtered_live(&raw_contents);
    let mut collapser = ReplayToolCollapser::new();
    let mut forwarded = false;
    for line in live {
        if let Some(update) = replay_line(line, &mut collapser, ReplayCorruptionPolicy::SkipMalformed)
            .map_err(io::Error::other)?
        {
            if matches!(&update, ReplayedUpdate::Acp(_, _)) {
                forwarded = true;
            }
            f(update);
        }
    }
    for update in collapser.take_pending() {
        forwarded = true;
        f(ReplayedUpdate::Acp(update, None));
    }
    Ok(if forwarded {
        ReplayEmission::Emitted
    } else {
        ReplayEmission::Empty
    })
}

/// Typed-load core: rewind-filter and forward every ACP update (including ACU).
/// Not used by [`stream_replay_updates_at`] (that path peeks and collapses).
pub(crate) fn for_each_replay_update_in_file<F: FnMut(acp::SessionUpdate)>(
    updates_path: &std::path::Path,
    mut f: F,
) -> std::io::Result<bool> {
    let raw_contents = std::fs::read_to_string(updates_path)?;
    let live = rewind_filtered_live(&raw_contents);
    let mut forwarded = false;
    for line in live {
        match SessionUpdateEnvelope::from_str(line) {
            Ok(SessionUpdate::Acp(notif)) => {
                forwarded = true;
                f(strip_context_wrappers(notif.update));
            }
            Ok(SessionUpdate::Xai(_)) => {}
            Err(e) => tracing::debug!(error = %e, "skipping unparseable replay line"),
        }
    }
    Ok(forwarded)
}

fn rewind_filtered_live(raw: &str) -> Vec<&str> {
    filter_rewind_lines(raw.lines().filter(|l| !l.trim().is_empty()).collect())
}

/// Unpaired spawns across the rewind-filtered timeline.
/// The substring pre-filter keeps non-subagent lines off the JSON path.
pub(crate) fn collect_unfinished_subagents(filtered: &[&str]) -> Vec<UnfinishedSubagent> {
    let mut pending: std::collections::BTreeMap<String, UnfinishedSubagent> =
        std::collections::BTreeMap::new();
    for line in filtered {
        if !line.contains("subagent_spawned") && !line.contains("subagent_finished") {
            continue;
        }
        let raw = serde_json::from_str::<RawLinePeek<'_>>(line)
            .ok()
            .and_then(|e| e.params.map(|p| p.get()))
            .unwrap_or(line);
        let Ok(notification) = serde_json::from_str::<SessionNotification>(raw) else {
            continue;
        };
        match notification.update {
            XaiUpdate::SubagentSpawned {
                subagent_id,
                attempt_id,
                child_session_id,
                ..
            } => {
                pending.insert(
                    subagent_id.clone(),
                    UnfinishedSubagent {
                        subagent_id,
                        attempt_id,
                        child_session_id,
                    },
                );
            }
            XaiUpdate::SubagentFinished { subagent_id, .. } => {
                pending.remove(&subagent_id);
            }
            _ => {}
        }
    }
    pending.into_values().collect()
}

/// The raw `_meta` object of a persisted line, if any, without allocating a `serde_json::Value`.
/// Handles both the enveloped (`{method,params}`) and legacy (params-at-top-level) on-disk formats.
fn line_meta(line: &str) -> Option<&serde_json::value::RawValue> {
    let env = serde_json::from_str::<RawLinePeek<'_>>(line).ok()?;
    let raw = env.params.map(|p| p.get()).unwrap_or(line);
    serde_json::from_str::<RawParamsPeek<'_>>(raw).ok()?.meta
}

/// Catalog lines stay on disk but are re-advertised after every `session/load`, so replay skips them.
/// The typed peek ignores the huge `availableCommands` array.
pub(crate) fn line_is_available_commands_update(line: &str) -> bool {
    line.contains(&*AVAILABLE_COMMANDS_UPDATE)
        && peek_line_update(line).is_some_and(|u| u.session_update == *AVAILABLE_COMMANDS_UPDATE)
}

/// Fat 100ms bash `tool_call_update`s: typed peek of `sessionUpdate` and `status` only (unknown fields ignored).
/// Completed/Failed and `status: None` stay.
pub(crate) fn line_is_in_progress_tool_call_update(line: &str) -> bool {
    if !line.contains(&*TOOL_CALL_UPDATE) || !line.contains(&*TOOL_CALL_STATUS_IN_PROGRESS) {
        return false;
    }
    peek_line_update(line).is_some_and(|u| {
        u.session_update == *TOOL_CALL_UPDATE
            && u.status == Some(TOOL_CALL_STATUS_IN_PROGRESS.as_str())
    })
}

fn peek_line_update(line: &str) -> Option<RawUpdatePeek<'_>> {
    let env = serde_json::from_str::<RawLinePeek<'_>>(line).ok()?;
    let raw = env.params.map(|p| p.get()).unwrap_or(line);
    serde_json::from_str::<RawParamsPeek<'_>>(raw).ok()?.update
}

pub(crate) fn line_is_dropped_on_replay(line: &str) -> bool {
    line_is_available_commands_update(line) || line_is_in_progress_tool_call_update(line)
}

fn replay_line(
    line: &str,
    tools: &mut ReplayToolCollapser,
    corruption: ReplayCorruptionPolicy,
) -> Result<Option<ReplayedUpdate>, PassiveReplayError> {
    if line_is_dropped_on_replay(line) {
        return Ok(None);
    }
    match SessionUpdateEnvelope::from_str(line) {
        Ok(SessionUpdate::Acp(notif)) => {
            let notif = *notif;
            Ok(tools.push(strip_context_wrappers(notif.update))
                .map(|update| ReplayedUpdate::Acp(update, notif.meta)))
        }
        Ok(SessionUpdate::Xai(notif)) => Ok(Some(ReplayedUpdate::Xai(notif.update, notif.meta))),
        Err(_) => match corruption {
            ReplayCorruptionPolicy::RejectMalformed => Err(PassiveReplayError::MalformedRecord),
            ReplayCorruptionPolicy::SkipMalformed => Ok(None),
        },
    }
}

/// Extract `_meta.totalTokens` from a persisted update line without allocating a `serde_json::Value`.
fn line_total_tokens(line: &str) -> Option<u64> {
    if !line.contains(TOTAL_TOKENS_KEY) {
        return None;
    }
    #[derive(serde::Deserialize)]
    struct TokensPeek {
        #[serde(rename = "totalTokens")]
        total_tokens: Option<u64>,
    }
    serde_json::from_str::<TokensPeek>(line_meta(line)?.get())
        .ok()
        .and_then(|t| t.total_tokens)
}

/// This line's `_meta.eventId`, if any. Cheap peek (no `Value`).
fn line_event_id(line: &str) -> Option<std::borrow::Cow<'_, str>> {
    if !line.contains(EVENT_ID_KEY) {
        return None;
    }
    #[derive(serde::Deserialize)]
    struct EventIdPeek<'a> {
        #[serde(rename = "eventId", borrow)]
        event_id: Option<std::borrow::Cow<'a, str>>,
    }
    serde_json::from_str::<EventIdPeek<'_>>(line_meta(line)?.get())
        .ok()
        .and_then(|e| e.event_id)
}

fn line_has_event_id(line: &str, cursor_id: &str) -> bool {
    line_event_id(line).as_deref() == Some(cursor_id)
}

/// Rewind-filter, resolve the reconnect cursor, drop redundant command catalogs and InProgress tool_call_updates, and scan `totalTokens`.
/// The cursor is resolved before dropping ACUs / InProgress lines: an idle client often reconnects with one of those `eventId`s as its cursor.
/// Resolving against the inclusive set keeps reconnect incremental instead of a full replay.
#[doc(hidden)]
pub fn prepare_replay_lines<'a>(contents: &'a str, cursor: Option<&str>) -> PreparedReplay<'a> {
    let filtered = filter_rewind_lines(contents.lines().filter(|l| !l.trim().is_empty()).collect());

    let mut max_event_seq: Option<u64> = None;
    for line in &filtered {
        if line.contains("eventId")
            && let Ok(env) = serde_json::from_str::<RawLinePeek<'_>>(line)
            && let Some(raw) = env.params.map(|p| p.get())
            && let Ok(pp) = serde_json::from_str::<RawParamsPeek<'_>>(raw)
            && let Some(meta_raw) = pp.meta
            && let Ok(meta) = serde_json::from_str::<serde_json::Value>(meta_raw.get())
            && let Some(seq) = meta
                .get("eventId")
                .and_then(|v| v.as_str())
                .and_then(|s| s.rsplit('-').next())
                .and_then(|c| c.parse::<u64>().ok())
        {
            max_event_seq = Some(max_event_seq.map_or(seq, |m| m.max(seq)));
        }
    }

    let last_tokens = filtered
        .iter()
        .rev()
        .find_map(|l| line_total_tokens(l))
        .unwrap_or(0);

    let mut selection = cursor
        .and_then(|id| filtered.iter().rposition(|line| line_has_event_id(line, id)))
        .map_or(ReplaySelection::NotFound, |position| ReplaySelection::After { next: position + 1 });
    if let ReplaySelection::After { next } = selection
        && let Some(cursor) = cursor
    {
        for (index, line) in filtered.iter().enumerate().skip(next) {
            selection.observe(line, index, cursor);
        }
    }
    if matches!(selection, ReplaySelection::Unbounded) {
        tracing::warn!("replay: post-cursor tail contains eventId-less lines; full replay instead");
    }
    let (start, replay_start) = selection.resolve();
    let mark_replay = matches!(replay_start, ReplayStart::Full);

    let mut lines: Vec<&str> = Vec::with_capacity(filtered.len().saturating_sub(start));
    let mut total_live = 0usize;
    for (i, &line) in filtered.iter().enumerate() {
        if line_is_dropped_on_replay(line) {
            continue;
        }
        total_live += 1;
        if i >= start {
            lines.push(line);
        }
    }

    PreparedReplay {
        lines,
        mark_replay,
        last_tokens,
        max_event_seq,
        total_live,
        unfinished_subagents: collect_unfinished_subagents(&filtered),
    }
}

/// Blank-strip, drop redundant command catalogs and InProgress tool updates, and rewind-filter a raw `updates.jsonl` segment.
/// Shared by the delta-replay path (which has no reconnect cursor).
/// The initial replay path is [`prepare_replay_lines`].
pub(crate) fn filter_delta_replay_lines(contents: &str) -> Vec<&str> {
    let live: Vec<&str> = contents
        .lines()
        .filter(|l| !l.trim().is_empty() && !line_is_dropped_on_replay(l))
        .collect();
    filter_rewind_lines(live)
}

/// All limits are caller-selected, nonzero bounds. A record's serde/merge work is
/// indivisible and may cost up to `max_record_bytes`; IO and marker visits yield.
#[derive(Clone, Copy, Debug)]
pub struct PassiveReplayLimits {
    pub max_record_bytes: std::num::NonZeroUsize,
}

#[derive(Clone, Copy, Debug)]
pub struct ReplayBudget {
    pub bytes: std::num::NonZeroUsize,
    /// Includes retention visits, tombstones, record parsing and phase transitions.
    pub records: std::num::NonZeroUsize,
    pub emits: std::num::NonZeroUsize,
}

#[derive(Clone, Copy, Debug)]
pub enum ReplayCorruptionPolicy {
    RejectMalformed,
    /// Matches established replay: malformed JSON/notifications are not forwarded.
    SkipMalformed,
}

#[derive(Clone, Copy, Debug)]
pub enum ReplayFinalRecordPolicy {
    /// Like `str::lines`, parse an unterminated final record (including valid JSON).
    Parse,
    /// The captured EOF is a cut; an unterminated suffix is not a committed record.
    IgnoreUnterminated,
}

/// No paths, transcript data or serde diagnostics escape this boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassiveReplayError {
    Io(io::ErrorKind),
    SnapshotChanged,
    RecordTooLarge,
    InvalidUtf8,
    MalformedRecord,
    NotFinished,
    MissingOrAmbiguous,
    Failed,
}

impl From<io::Error> for PassiveReplayError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

impl std::fmt::Display for PassiveReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "passive replay: {self:?}")
    }
}

impl std::error::Error for PassiveReplayError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassiveReplayPhase {
    Planning,
    Selecting,
    Emitting,
    Flushing,
    Verifying,
    Finishing,
    Finished,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayCompletion {
    pub emission: ReplayEmission,
    pub start: ReplayStart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayStart {
    Full,
    AfterCursor,
}

pub struct PassiveReplayBatch {
    pub updates: Vec<ReplayedUpdate>,
    pub phase: PassiveReplayPhase,
    /// IO bytes plus bytes consumed by the incremental line scanner.
    pub bytes_used: usize,
    pub records_used: usize,
}

impl std::fmt::Debug for PassiveReplayBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassiveReplayBatch")
            .field("updates", &self.updates.len())
            .field("phase", &self.phase)
            .field("bytes_used", &self.bytes_used)
            .field("records_used", &self.records_used)
            .finish()
    }
}

#[derive(PartialEq, Eq)]
struct ReplayFileEpoch {
    length: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64),
    #[cfg(unix)]
    changed: (i64, i64),
}

impl ReplayFileEpoch {
    fn read(file: &std::fs::File) -> Result<Self, PassiveReplayError> {
        let metadata = file.metadata()?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            identity: (metadata.dev(), metadata.ino()),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

#[derive(Clone, Copy)]
struct ReplayRecord {
    offset: u64,
    length: u64,
    digest: blake3::Hash,
    observation: ReplayCursorObservation,
}

const REPLAY_IO_CHUNK_BYTES: usize = 8192;

struct ReplayLineReader {
    end: u64,
    read_at: u64,
    line_at: u64,
    buffer: [u8; REPLAY_IO_CHUNK_BYTES],
    next: usize,
    filled: usize,
    line: Vec<u8>,
    digest: blake3::Hasher,
}

enum ReplayLineProgress {
    Yield,
    Record(ReplayRecord, Vec<u8>),
    End,
}

impl ReplayLineReader {
    fn new(start: u64, end: u64) -> Self {
        Self { end, read_at: start, line_at: start, buffer: [0; REPLAY_IO_CHUNK_BYTES], next: 0, filled: 0, line: Vec::new(), digest: blake3::Hasher::new() }
    }

    fn next(
        &mut self,
        file: &mut std::fs::File,
        bytes: &mut usize,
        limit: usize,
        final_record: ReplayFinalRecordPolicy,
    ) -> Result<ReplayLineProgress, PassiveReplayError> {
        use std::io::{Read, Seek};
        loop {
            if self.next < self.filled {
                if *bytes == 0 {
                    return Ok(ReplayLineProgress::Yield);
                }
                let byte = self.buffer[self.next];
                self.next += 1;
                *bytes -= 1;
                if byte == b'\n' {
                    let offset = self.line_at;
                    let consumed = u64::try_from(self.filled - self.next).map_err(|_| PassiveReplayError::RecordTooLarge)?;
                    self.line_at = self.read_at - consumed;
                    return Ok(ReplayLineProgress::Record(
                        ReplayRecord { offset, length: self.line_at - offset, digest: blake3::hash(&self.line), observation: ReplayCursorObservation::Other },
                        std::mem::take(&mut self.line),
                    ));
                }
                if self.line.len() == limit {
                    return Err(PassiveReplayError::RecordTooLarge);
                }
                self.line.push(byte);
            } else if self.read_at == self.end {
                if self.line.is_empty() {
                    return Ok(ReplayLineProgress::End);
                }
                return match final_record {
                    ReplayFinalRecordPolicy::Parse => Ok(ReplayLineProgress::Record(
                        ReplayRecord { offset: self.line_at, length: self.end - self.line_at, digest: blake3::hash(&self.line), observation: ReplayCursorObservation::Other },
                        std::mem::take(&mut self.line),
                    )),
                    ReplayFinalRecordPolicy::IgnoreUnterminated => {
                        self.line.clear();
                        Ok(ReplayLineProgress::End)
                    }
                };
            } else {
                if *bytes == 0 {
                    return Ok(ReplayLineProgress::Yield);
                }
                let chunk = u64::try_from(self.buffer.len()).map_err(|_| PassiveReplayError::RecordTooLarge)?;
                let available = usize::try_from((self.end - self.read_at).min(chunk))
                    .map_err(|_| PassiveReplayError::RecordTooLarge)?;
                let count = available.min(*bytes);
                file.seek(io::SeekFrom::Start(self.read_at))?;
                let count = file.read(&mut self.buffer[..count])?;
                if count == 0 {
                    return Err(PassiveReplayError::SnapshotChanged);
                }
                self.digest.update(&self.buffer[..count]);
                self.read_at += u64::try_from(count).map_err(|_| PassiveReplayError::RecordTooLarge)?;
                *bytes -= count;
                self.next = 0;
                self.filled = count;
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
enum ReplaySelection {
    #[default]
    NotFound,
    After { next: usize },
    Unbounded,
}

#[derive(Clone, Copy)]
enum ReplayCursorObservation {
    Match,
    Unbounded,
    Other,
}

impl ReplayCursorObservation {
    fn from_line(line: &str, cursor: &str) -> Self {
        if line_has_event_id(line, cursor) {
            Self::Match
        } else if !line_is_dropped_on_replay(line) && line_event_id(line).is_none() {
            Self::Unbounded
        } else {
            Self::Other
        }
    }
}

impl ReplaySelection {
    fn observe(&mut self, line: &str, index: usize, cursor: &str) {
        self.observe_record(ReplayCursorObservation::from_line(line, cursor), index);
    }

    fn observe_record(&mut self, observation: ReplayCursorObservation, index: usize) {
        match observation {
            ReplayCursorObservation::Match => *self = Self::After { next: index + 1 },
            ReplayCursorObservation::Unbounded => match self {
                Self::NotFound => {}
                Self::After { .. } | Self::Unbounded => *self = Self::Unbounded,
            },
            ReplayCursorObservation::Other => {}
        }
    }

    fn resolve(self) -> (usize, ReplayStart) {
        match self {
            Self::NotFound | Self::Unbounded => (0, ReplayStart::Full),
            Self::After { next } => (next, ReplayStart::AfterCursor),
        }
    }
}

struct ReplayPrefixProof {
    end: u64,
    digest: blake3::Hash,
}

struct PassiveEmission {
    proof: ReplayPrefixProof,
    tools: ReplayToolCollapser,
    completion: ReplayCompletion,
}

enum PassiveState {
    Planning(Box<ReplayLineReader>),
    Selecting { next: usize, selection: ReplaySelection, proof: ReplayPrefixProof },
    Emitting { next: usize, emission: PassiveEmission },
    EmitRead { index: usize, reader: Box<ReplayLineReader>, emission: PassiveEmission },
    Flushing(PassiveEmission),
    Verifying { reader: Box<ReplayLineReader>, expected: blake3::Hash, completion: ReplayCompletion },
    Finishing(ReplayCompletion),
    Finished(ReplayCompletion),
    Faulted { error: PassiveReplayError, _suspended: Box<PassiveState> },
    Failed,
}

/// Storage-only, Send-owned replay. It holds an open read-only descriptor and an
/// offset/tombstone plan, not another transcript. No callback or runtime is started.
///
/// The descriptor's EOF at open is a fixed fence. Storage's same-inode writer
/// appends (including torn-tail healing); full rewrites atomically replace the inode.
/// Growth never extends the fence or restarts progress. Observed shrink or same-size
/// metadata changes fail. Planning hashes every fenced byte; emission checks each
/// retained record and a final budgeted pass verifies the entire prefix, including
/// removed records and ignored tails. Growth cannot waive these integrity checks.
/// This is not an atomic filesystem snapshot: arbitrary in-place races during or
/// after verification are outside the append-only storage contract and may escape
/// observation. Rename keeps the opened inode rather than retargeting the replay.
/// Stage batches until `finish` succeeds; an error invalidates the entire replay.
/// Dropping cancels without emitting; cancellation/fault destruction is O(plan),
/// so dispose of abandoned cursors off-UI. No Pager production caller exists yet.
/// Planning, retained emission and integrity verification cost 2–3 passes' IO;
/// cursor selection uses stored observations rather than rereading payloads.
/// Syscalls, vector growth and one bounded record's serde/hash/merge are indivisible.
pub struct PassiveReplayCursor {
    file: std::fs::File,
    epoch: ReplayFileEpoch,
    limits: PassiveReplayLimits,
    corruption: ReplayCorruptionPolicy,
    final_record: ReplayFinalRecordPolicy,
    cursor: Option<String>,
    retained: super::RewindSurvivors<ReplayRecord>,
    state: PassiveState,
}

impl PassiveReplayCursor {
    /// Validated storage lookup, including relocation when requested by `hint`.
    /// Run open off-UI: relocation may scan the sessions root. No lookup occurs in
    /// `next_batch`. The shared resolver intentionally conflates missing/ambiguous.
    pub fn open_at(
        session_id: &str,
        grok_home: &Path,
        hint: ReplayPathHint<'_>,
        limits: PassiveReplayLimits,
        corruption: ReplayCorruptionPolicy,
        final_record: ReplayFinalRecordPolicy,
        cursor: Option<String>,
    ) -> Result<Self, PassiveReplayError> {
        if !is_safe_session_id_component(session_id) || session_id.contains('\\') {
            return Err(PassiveReplayError::MissingOrAmbiguous);
        }
        let path = resolve_replay_updates_path(session_id, grok_home, hint)?
            .ok_or(PassiveReplayError::MissingOrAmbiguous)?;
        Self::open(&path, limits, corruption, final_record, cursor)
    }

    /// `path` must come from the existing validated storage resolver. This performs
    /// only open/stat; keep opening and eventual disposal off-UI.
    pub(crate) fn open(
        path: &Path,
        limits: PassiveReplayLimits,
        corruption: ReplayCorruptionPolicy,
        final_record: ReplayFinalRecordPolicy,
        cursor: Option<String>,
    ) -> Result<Self, PassiveReplayError> {
        let file = std::fs::File::open(path)?;
        let epoch = ReplayFileEpoch::read(&file)?;
        let end = epoch.length;
        Ok(Self {
            file, epoch, limits, corruption, final_record, cursor,
            retained: super::RewindSurvivors::new(),
            state: PassiveState::Planning(Box::new(ReplayLineReader::new(0, end))),
        })
    }

    pub fn phase(&self) -> PassiveReplayPhase {
        match &self.state {
            PassiveState::Planning(_) => PassiveReplayPhase::Planning,
            PassiveState::Selecting { .. } => PassiveReplayPhase::Selecting,
            PassiveState::Emitting { .. } | PassiveState::EmitRead { .. } => PassiveReplayPhase::Emitting,
            PassiveState::Flushing(_) => PassiveReplayPhase::Flushing,
            PassiveState::Verifying { .. } => PassiveReplayPhase::Verifying,
            PassiveState::Finishing(_) => PassiveReplayPhase::Finishing,
            PassiveState::Finished(_) => PassiveReplayPhase::Finished,
            PassiveState::Faulted { .. } | PassiveState::Failed => PassiveReplayPhase::Failed,
        }
    }

    fn check_epoch(&self) -> Result<ReplayFileEpoch, PassiveReplayError> {
        let current = ReplayFileEpoch::read(&self.file)?;
        #[cfg(unix)]
        if current.identity != self.epoch.identity {
            return Err(PassiveReplayError::SnapshotChanged);
        }
        if current.length < self.epoch.length
            || (current.length == self.epoch.length && current != self.epoch)
        {
            return Err(PassiveReplayError::SnapshotChanged);
        }
        Ok(current)
    }

    /// Budgets bound IO/scanning bytes, positional/record visits and emitted items,
    /// not wall time. Two metadata reads and bounded record serde are indivisible.
    pub fn next_batch(&mut self, budget: ReplayBudget) -> Result<PassiveReplayBatch, PassiveReplayError> {
        match &self.state {
            PassiveState::Faulted { error, .. } => return Err(*error),
            PassiveState::Failed => return Err(PassiveReplayError::Failed),
            PassiveState::Planning(_) | PassiveState::Selecting { .. }
            | PassiveState::Emitting { .. } | PassiveState::EmitRead { .. } | PassiveState::Flushing(_)
            | PassiveState::Verifying { .. } | PassiveState::Finishing(_) | PassiveState::Finished(_) => {}
        }
        let result = self.advance_batch(budget);
        if let Err(error) = &result {
            let suspended = std::mem::replace(&mut self.state, PassiveState::Failed);
            self.state = PassiveState::Faulted { error: *error, _suspended: Box::new(suspended) };
        }
        result
    }

    fn advance_batch(&mut self, budget: ReplayBudget) -> Result<PassiveReplayBatch, PassiveReplayError> {
        self.epoch = self.check_epoch()?;
        let mut bytes = budget.bytes.get();
        let mut records = budget.records.get();
        let mut emits = budget.emits.get();
        let mut updates = Vec::new();
        while records > 0 && emits > 0 {
            let state = std::mem::replace(&mut self.state, PassiveState::Failed);
            self.state = match state {
                PassiveState::Planning(mut reader) => {
                    if !self.retained.is_idle() {
                        self.retained.advance();
                        records -= 1;
                        PassiveState::Planning(reader)
                    } else {
                        match reader.next(&mut self.file, &mut bytes, self.limits.max_record_bytes.get(), self.final_record)? {
                            ReplayLineProgress::Yield => {
                                self.state = PassiveState::Planning(reader);
                                break;
                            }
                            ReplayLineProgress::End => {
                                records -= 1;
                                let proof = ReplayPrefixProof { end: reader.end, digest: reader.digest.finalize() };
                                if self.cursor.is_some() {
                                    PassiveState::Selecting { next: 0, selection: ReplaySelection::default(), proof }
                                } else {
                                    PassiveState::Emitting {
                                        next: 0,
                                        emission: PassiveEmission {
                                            proof,
                                            tools: ReplayToolCollapser::new(),
                                            completion: ReplayCompletion { emission: ReplayEmission::Empty, start: ReplayStart::Full },
                                        },
                                    }
                                }
                            }
                            ReplayLineProgress::Record(mut record, raw) => {
                                records -= 1;
                                let line = std::str::from_utf8(&raw).map_err(|_| PassiveReplayError::InvalidUtf8)?;
                                if !line.trim().is_empty() {
                                    if matches!(self.corruption, ReplayCorruptionPolicy::RejectMalformed)
                                        && SessionUpdateEnvelope::from_str(line).is_err()
                                    {
                                        return Err(PassiveReplayError::MalformedRecord);
                                    }
                                    if let Some(cursor) = self.cursor.as_deref() {
                                        record.observation = ReplayCursorObservation::from_line(line, cursor);
                                    }
                                    self.retained.push(record, super::rewind_step_for_line(line));
                                }
                                PassiveState::Planning(reader)
                            }
                        }
                    }
                }
                PassiveState::Selecting { next, mut selection, proof } => {
                    records -= 1;
                    match self.retained.items.get(next) {
                        Some(super::Survivor::Removed) => PassiveState::Selecting { next: next + 1, selection, proof },
                        Some(super::Survivor::Kept(record)) => {
                            selection.observe_record(record.observation, next);
                            PassiveState::Selecting { next: next + 1, selection, proof }
                        }
                        None => {
                            let (next, start) = selection.resolve();
                            PassiveState::Emitting {
                                next,
                                emission: PassiveEmission {
                                    proof,
                                    tools: ReplayToolCollapser::new(),
                                    completion: ReplayCompletion { emission: ReplayEmission::Empty, start },
                                },
                            }
                        }
                    }
                }
                PassiveState::Emitting { next, emission } => {
                    records -= 1;
                    match self.retained.items.get(next) {
                        Some(super::Survivor::Removed) => PassiveState::Emitting { next: next + 1, emission },
                        Some(super::Survivor::Kept(record)) => PassiveState::EmitRead {
                            index: next, reader: Box::new(ReplayLineReader::new(record.offset, record.offset + record.length)), emission,
                        },
                        None => PassiveState::Flushing(emission),
                    }
                }
                PassiveState::EmitRead { index, mut reader, mut emission } => {
                    let progress = match reader.next(&mut self.file, &mut bytes, self.limits.max_record_bytes.get(), ReplayFinalRecordPolicy::Parse) {
                        Ok(progress) => progress,
                        Err(error) => {
                            self.state = PassiveState::EmitRead { index, reader, emission };
                            return Err(error);
                        }
                    };
                    match progress {
                        ReplayLineProgress::Yield => {
                            self.state = PassiveState::EmitRead { index, reader, emission };
                            break;
                        }
                        ReplayLineProgress::End => {
                            self.state = PassiveState::EmitRead { index, reader, emission };
                            return Err(PassiveReplayError::SnapshotChanged);
                        },
                        ReplayLineProgress::Record(read_record, raw) => {
                            records -= 1;
                            if let Some(super::Survivor::Kept(record)) = self.retained.items.get(index) {
                                if read_record.digest != record.digest {
                                    self.state = PassiveState::EmitRead { index, reader, emission };
                                    return Err(PassiveReplayError::SnapshotChanged);
                                }
                            }
                            let line = match std::str::from_utf8(&raw) {
                                Ok(line) => line,
                                Err(_) => {
                                    self.state = PassiveState::EmitRead { index, reader, emission };
                                    return Err(PassiveReplayError::InvalidUtf8);
                                }
                            };
                            match replay_line(line, &mut emission.tools, self.corruption) {
                                Ok(Some(update)) => {
                                    if matches!(&update, ReplayedUpdate::Acp(_, _)) {
                                        emission.completion.emission = ReplayEmission::Emitted;
                                    }
                                    updates.push(update);
                                    emits -= 1;
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    self.state = PassiveState::EmitRead { index, reader, emission };
                                    return Err(error);
                                }
                            }
                            PassiveState::Emitting { next: index + 1, emission }
                        }
                    }
                }
                PassiveState::Flushing(mut emission) => {
                    records -= 1;
                    if let Some(update) = emission.tools.pop_pending() {
                        emission.completion.emission = ReplayEmission::Emitted;
                        updates.push(ReplayedUpdate::Acp(update, None));
                        emits -= 1;
                        PassiveState::Flushing(emission)
                    } else {
                        PassiveState::Verifying {
                            reader: Box::new(ReplayLineReader::new(0, emission.proof.end)),
                            expected: emission.proof.digest,
                            completion: emission.completion,
                        }
                    }
                }
                PassiveState::Verifying { mut reader, expected, completion } => {
                    match reader.next(&mut self.file, &mut bytes, self.limits.max_record_bytes.get(), self.final_record)? {
                        ReplayLineProgress::Yield => {
                            self.state = PassiveState::Verifying { reader, expected, completion };
                            break;
                        }
                        ReplayLineProgress::Record(_, _) => {
                            records -= 1;
                            PassiveState::Verifying { reader, expected, completion }
                        }
                        ReplayLineProgress::End => {
                            records -= 1;
                            if reader.digest.finalize() != expected {
                                return Err(PassiveReplayError::SnapshotChanged);
                            }
                            PassiveState::Finishing(completion)
                        }
                    }
                }
                PassiveState::Finishing(completion) => {
                    records -= 1;
                    if self.retained.items.pop().is_some()
                        || self.retained.turns.pop().is_some()
                        || self.retained.tool_updates.pop().is_some()
                    {
                        PassiveState::Finishing(completion)
                    } else {
                        PassiveState::Finished(completion)
                    }
                }
                PassiveState::Finished(completion) => {
                    self.state = PassiveState::Finished(completion);
                    break;
                }
                PassiveState::Faulted { error, _suspended } => {
                    self.state = PassiveState::Faulted { error, _suspended };
                    return Err(error);
                }
                PassiveState::Failed => return Err(PassiveReplayError::Failed),
            };
        }
        self.epoch = self.check_epoch()?;
        Ok(PassiveReplayBatch {
            updates, phase: self.phase(), bytes_used: budget.bytes.get() - bytes,
            records_used: budget.records.get() - records,
        })
    }

    /// A successful finish is the only commit point for staged consumer updates.
    pub fn finish(&self) -> Result<ReplayCompletion, PassiveReplayError> {
        self.check_epoch()?;
        match &self.state {
            PassiveState::Finished(completion) => Ok(*completion),
            PassiveState::Faulted { error, .. } => Err(*error),
            PassiveState::Failed => Err(PassiveReplayError::Failed),
            PassiveState::Planning(_) | PassiveState::Selecting { .. }
            | PassiveState::Emitting { .. } | PassiveState::EmitRead { .. } | PassiveState::Flushing(_)
            | PassiveState::Verifying { .. } | PassiveState::Finishing(_) => Err(PassiveReplayError::NotFinished),
        }
    }
}
