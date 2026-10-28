use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::jsonl::history_decoder::{HistoryDecodeLimit, HistoryIssue, HistoryLimits, LegacyReasoningIssue, decode_history_bounded};
use super::update_decoder::{UpdateMethod, decode_update};
use super::{CHAT_HISTORY_FILE, SUMMARY_FILE, SessionUpdate, UPDATES_FILE};
use crate::agent::subagent::SubagentMeta;
use crate::session::persistence::{CHAT_FORMAT_VERSION, Summary};
use crate::session::request_capture::RequestSnapshotId;
use xai_grok_session_events::{Event, decode_event_entry};
use xai_grok_workspace::session::file_state::RewindPoint;

#[path = "audit_budget.rs"]
mod budget;
pub use budget::{AuditLimits, BudgetExceeded, BudgetKind, MAX_INVENTORY_ENTRIES, MAX_ISSUES, MAX_RESPONSE_BYTES};
use budget::{Budget, InventoryBudget};

pub const PROTOCOL: u32 = 1;
pub const CAPABILITY: &str = "grok-native-session-audit-v1";

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Summary,
    History,
    Updates,
    Events,
    Rewind,
    RequestIndex,
    SubagentMeta,
    Opaque,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct AuditFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub kind: FileKind,
    pub records: usize,
    pub blank_lines: usize,
    pub unterminated_final_lines: usize,
}

#[derive(Serialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum IssueCause {
    Inventory,
    Unreadable,
    Changed,
    Empty,
    MissingSummary,
    InvalidSummary,
    UnknownFormatVersion,
    IdentityMismatch,
    InvalidRecord { line: usize },
    InvalidJson { line: usize },
    InvalidImage { line: usize, part: usize },
    InvalidLegacyReasoning { line: usize },
    InvalidRawOutput { line: usize, index: usize },
    UnknownUpdate { line: usize },
    InvalidTimestamp { line: usize },
    UnknownEventVersion { line: usize },
    InvalidMeta,
}

#[derive(Serialize)]
pub struct AuditIssue {
    pub path: String,
    #[serde(flatten)]
    pub cause: IssueCause,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditResult {
    SupportedNoTransform,
    Refused,
}

#[derive(Serialize)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum AuditResponse<'a> {
    Capability { protocol: u32, capability: &'static str, build: &'a str },
    Incomplete { protocol: u32, capability: &'static str, build: &'a str, budget: BudgetKind, observed_files: usize, observed_issues: usize },
    Audit { protocol: u32, capability: &'static str, build: &'a str, result: AuditResult, files: Vec<AuditFile>, issues: Vec<AuditIssue> },
}

pub fn capability(build: &str) -> AuditResponse<'_> {
    AuditResponse::Capability { protocol: PROTOCOL, capability: CAPABILITY, build }
}

struct Findings {
    entries: Vec<AuditIssue>,
    budget: Budget,
}

fn issue(issues: &mut Findings, path: &str, cause: IssueCause) -> Result<(), BudgetExceeded> {
    issues.budget.issue(path)?;
    issues.entries.push(AuditIssue { path: path.to_owned(), cause });
    Ok(())
}

enum InventoryError {
    Io,
    Limit(BudgetKind),
}

impl From<io::Error> for InventoryError {
    fn from(_: io::Error) -> Self { Self::Io }
}

#[derive(PartialEq, Eq)]
struct Inventory {
    files: BTreeMap<String, FileKind>,
    state: BTreeMap<String, FileState>,
    sessions: Vec<String>,
}

#[derive(PartialEq, Eq)]
struct FileState {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, u64, i64, i64, u32),
}

fn state(meta: &fs::Metadata) -> FileState {
    FileState {
        len: meta.len(),
        modified: meta.modified().ok(),
        #[cfg(unix)]
        identity: {
            use std::os::unix::fs::MetadataExt;
            (meta.dev(), meta.ino(), meta.nlink(), meta.ctime(), meta.ctime_nsec(), meta.mode())
        },
    }
}

fn classify(parts: &[&str]) -> FileKind {
    match parts {
        [_, _, SUMMARY_FILE] => FileKind::Summary,
        [_, _, CHAT_HISTORY_FILE] => FileKind::History,
        [_, _, UPDATES_FILE] => FileKind::Updates,
        [_, _, "events.jsonl"] => FileKind::Events,
        [_, _, "rewind_points.jsonl"] => FileKind::Rewind,
        [_, _, "request-capture-v1", "requests.jsonl"] => FileKind::RequestIndex,
        [_, _, "subagents", _, "meta.json"] => FileKind::SubagentMeta,
        _ => FileKind::Opaque,
    }
}

fn inventory(root: &Path, limits: AuditLimits) -> Result<Inventory, InventoryError> {
    let mut budget = InventoryBudget::new(limits);
    for ancestor in root.ancestors().filter(|path| !path.as_os_str().is_empty()) {
        if fs::symlink_metadata(ancestor)?.file_type().is_symlink() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput).into());
        }
    }
    if !root.is_dir() { return Err(io::Error::from(io::ErrorKind::NotADirectory).into()); }
    let mut inventory = Inventory { files: BTreeMap::new(), state: BTreeMap::new(), sessions: Vec::new() };
    for entry in walkdir::WalkDir::new(root).follow_links(false).max_open(limits.inventory_depth + 4) {
        let entry = entry.map_err(io::Error::other)?;
        if entry.depth() > limits.inventory_depth { return Err(InventoryError::Limit(BudgetKind::InventoryDepth)); }
        let relative = entry.path().strip_prefix(root).map_err(io::Error::other)?;
        budget.entry(relative.as_os_str().len()).map_err(InventoryError::Limit)?;
        let mut parts = Vec::new();
        for component in relative.components() {
            let part = component.as_os_str().to_str().ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            if part.contains('\\') || part.chars().any(char::is_control) { return Err(io::Error::from(io::ErrorKind::InvalidInput).into()); }
            parts.push(part);
        }
        let path = parts.join("/");
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.file_type().is_symlink() || (!meta.is_dir() && !meta.is_file()) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput).into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.is_file() && meta.nlink() != 1 { return Err(io::Error::from(io::ErrorKind::InvalidInput).into()); }
        }
        inventory.state.insert(path.clone(), state(&meta));
        if meta.is_dir() && parts.len() == 2 { inventory.sessions.push(path.clone()); }
        if meta.is_file() {
            let kind = classify(&parts);
            inventory.files.insert(path, kind);
        }
    }
    inventory.sessions.sort();
    Ok(inventory)
}

enum ReadMode {
    HashOnly,
    Decode { limit: u64 },
}

fn read_file(path: &Path, mode: ReadMode) -> io::Result<(String, u64, Vec<u8>)> {
    let mut options = File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    let before = file.metadata()?;
    if !before.is_file() { return Err(io::Error::from(io::ErrorKind::InvalidInput).into()); }
    let decode_limit = match mode { ReadMode::HashOnly => 0, ReadMode::Decode { limit } => limit };
    let mut digest = Sha256::new();
    let mut bytes = 0;
    let mut contents = Vec::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 { break; }
        let chunk = buffer.get(..count).ok_or(io::ErrorKind::InvalidData)?;
        digest.update(chunk);
        bytes += count as u64;
        if before.len() <= decode_limit && bytes <= decode_limit { contents.extend_from_slice(chunk); }
    }
    if state(&before) != state(&file.metadata()?) || bytes != before.len() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok((format!("{:x}", digest.finalize()), bytes, contents))
}

fn decode_stream(file: &mut AuditFile, contents: &[u8], version: u8, issues: &mut Findings) -> Result<(), BudgetExceeded> {
    if file.kind == FileKind::History {
        let remaining_issues = issues.budget.limits.issues.saturating_sub(issues.budget.observed_issues);
        let response_issues = issues.budget.issue_capacity(&file.path);
        let limits = HistoryLimits { items: issues.budget.limits.history_items, issues: remaining_issues.min(response_issues) };
        match decode_history_bounded(contents, version, limits) {
            Ok(history) => for finding in history.issues() {
                let cause = match finding {
                    HistoryIssue::InvalidJson { line } => IssueCause::InvalidJson { line },
                    HistoryIssue::InvalidRecord { line } => IssueCause::InvalidRecord { line },
                    HistoryIssue::InvalidImage { line, part, cause: _ } => IssueCause::InvalidImage { line, part },
                    HistoryIssue::InvalidLegacyReasoning { line, cause } => match cause {
                        LegacyReasoningIssue::InvalidOutputItem { index } => IssueCause::InvalidRawOutput { line, index },
                        LegacyReasoningIssue::InvalidRawOutput | LegacyReasoningIssue::InvalidReasoning | LegacyReasoningIssue::InvalidReasoningContent => IssueCause::InvalidLegacyReasoning { line },
                    },
                };
                issue(issues, &file.path, cause)?;
            },
            Err(HistoryDecodeLimit::Items { observed_issues, limit: _ }) => {
                issues.budget.observed_issues += observed_issues;
                return Err(issues.budget.exceeded(BudgetKind::HistoryItems));
            }
            Err(HistoryDecodeLimit::Issues { observed, line: _ }) => {
                issues.budget.observed_issues += observed;
                return Err(issues.budget.exceeded(if response_issues < remaining_issues { BudgetKind::ResponseBytes } else { BudgetKind::Issues }));
            }
        }
    }
    for (index, raw) in contents.split_inclusive(|b| *b == b'\n').enumerate() {
        let line = index + 1;
        if raw.trim_ascii().is_empty() {
            file.blank_lines += 1;
            if file.kind == FileKind::RequestIndex { issue(issues, &file.path, IssueCause::InvalidRecord { line })?; }
            continue;
        }
        file.records += 1;
        if !raw.ends_with(b"\n") { file.unterminated_final_lines += 1; }
        let valid = match file.kind {
            FileKind::History => true,
            FileKind::Updates => match std::str::from_utf8(raw).ok().and_then(|raw| decode_update(raw).ok()) {
                Some(decoded) => {
                    if matches!(decoded.method, UpdateMethod::Unknown | UpdateMethod::Invalid) || matches!(decoded.update, SessionUpdate::Xai(ref notification) if matches!(notification.update, crate::extensions::notification::SessionUpdate::Unknown)) {
                        issue(issues, &file.path, IssueCause::UnknownUpdate { line })?;
                    }
                    if !decoded.timestamp_valid { issue(issues, &file.path, IssueCause::InvalidTimestamp { line })?; }
                    true
                }
                None => false,
            },
            FileKind::Events => match std::str::from_utf8(raw).ok().and_then(|raw| decode_event_entry(raw).ok()) {
                Some(entry) => {
                    if let Event::TurnStarted { schema_version, .. } = entry.event
                        && schema_version != xai_grok_session_events::EVENT_SCHEMA_VERSION {
                        issue(issues, &file.path, IssueCause::UnknownEventVersion { line })?;
                    }
                    true
                }
                None => false,
            },
            FileKind::Rewind => serde_json::from_slice::<RewindPoint>(raw).is_ok(),
            FileKind::RequestIndex => serde_json::from_slice::<RequestSnapshotId>(raw).is_ok(),
            FileKind::Summary | FileKind::SubagentMeta | FileKind::Opaque => true,
        };
        if !valid { issue(issues, &file.path, IssueCause::InvalidRecord { line })?; }
    }
    Ok(())
}

pub fn audit<'a>(root: &Path, build: &'a str) -> AuditResponse<'a> {
    audit_with_limits(root, build, AuditLimits::default())
}

pub fn audit_with_limits<'a>(root: &Path, build: &'a str, limits: AuditLimits) -> AuditResponse<'a> {
    let mut findings = Findings { entries: Vec::new(), budget: Budget::new(limits, build) };
    match audit_files(root, &mut findings) {
        Ok(files) => {
            let result = if findings.entries.is_empty() { AuditResult::SupportedNoTransform } else { AuditResult::Refused };
            AuditResponse::Audit { protocol: PROTOCOL, capability: CAPABILITY, build, result, files, issues: findings.entries }
        }
        Err(limit) => AuditResponse::Incomplete { protocol: PROTOCOL, capability: CAPABILITY, build, budget: limit.budget, observed_files: limit.observed_files, observed_issues: limit.observed_issues },
    }
}

fn audit_files(root: &Path, issues: &mut Findings) -> Result<Vec<AuditFile>, BudgetExceeded> {
    issues.budget.response(0)?;
    let mut files = Vec::new();
    let before = match inventory(root, issues.budget.limits) {
        Ok(inventory) => inventory,
        Err(InventoryError::Io) => {
            issue(issues, ".", IssueCause::Inventory)?;
            return Ok(files);
        }
        Err(InventoryError::Limit(kind)) => return Err(issues.budget.exceeded(kind)),
    };
    if before.sessions.is_empty() { issue(issues, ".", IssueCause::Empty)?; }
    let mut summaries = BTreeMap::new();
    for session in &before.sessions {
        let path = format!("{session}/{SUMMARY_FILE}");
        if !before.files.contains_key(&path) { issue(issues, &path, IssueCause::MissingSummary)?; }
    }
    // Summaries establish the decoder version for every stream in their directory.
    let mut ordered: Vec<_> = before.files.iter().collect();
    ordered.sort_by_key(|(_, kind)| if **kind == FileKind::Summary { 0 } else { 1 });
    for (path, kind) in ordered {
        issues.budget.response(budget::file_bytes(path))?;
        let mode = match kind {
            FileKind::Opaque => ReadMode::HashOnly,
            FileKind::Summary | FileKind::History | FileKind::Updates | FileKind::Events | FileKind::Rewind | FileKind::RequestIndex | FileKind::SubagentMeta => ReadMode::Decode { limit: issues.budget.limits.decode_bytes },
        };
        let (sha256, bytes, contents) = match read_file(&root.join(path), mode) {
            Ok(read) => read,
            Err(_) => { issue(issues, path, IssueCause::Unreadable)?; continue; }
        };
        issues.budget.observed_files += 1;
        let mut file = AuditFile { path: path.clone(), sha256, bytes, kind: *kind, records: 0, blank_lines: 0, unterminated_final_lines: 0 };
        if *kind != FileKind::Opaque && bytes > issues.budget.limits.decode_bytes {
            return Err(issues.budget.exceeded(BudgetKind::DecodeBytes));
        } else {
            match kind {
                FileKind::Summary => match serde_json::from_slice::<Summary>(&contents) {
                    Ok(summary) => {
                        if summary.chat_format_version > CHAT_FORMAT_VERSION { issue(issues, path, IssueCause::UnknownFormatVersion)?; }
                        let parent = Path::new(path).parent().and_then(Path::to_str).unwrap_or("");
                        if Path::new(parent).file_name().and_then(|s| s.to_str()) != Some(summary.info.id.0.as_ref()) { issue(issues, path, IssueCause::IdentityMismatch)?; }
                        summaries.insert(parent.to_owned(), summary.chat_format_version);
                    }
                    Err(_) => issue(issues, path, IssueCause::InvalidSummary)?,
                },
                FileKind::SubagentMeta => match serde_json::from_slice::<SubagentMeta>(&contents) {
                    Ok(meta) => {
                        let parts: Vec<_> = path.split('/').collect();
                        let parent = parts.get(..2).map(|parts| parts.join("/")).unwrap_or_default();
                        if parts.get(3).copied() != Some(meta.subagent_id.as_str()) || summaries.get(&parent).is_none() || parts.get(1).copied() != Some(meta.parent_session_id.as_str()) {
                            issue(issues, path, IssueCause::IdentityMismatch)?;
                        }
                    }
                    Err(_) => issue(issues, path, IssueCause::InvalidMeta)?,
                },
                FileKind::Opaque => {},
                FileKind::History | FileKind::Updates | FileKind::Events | FileKind::Rewind | FileKind::RequestIndex => {
                    let parts: Vec<_> = path.split('/').collect();
                    let parent = parts.get(..2).map(|parts| parts.join("/")).unwrap_or_default();
                    let version = summaries.get(&parent).copied().unwrap_or(CHAT_FORMAT_VERSION);
                    decode_stream(&mut file, &contents, version, issues)?;
                }
            }
        }
        files.push(file);
    }
    match inventory(root, issues.budget.limits) {
        Ok(after) if after == before => {},
        Ok(_) | Err(InventoryError::Io) => issue(issues, ".", IssueCause::Changed)?,
        Err(InventoryError::Limit(kind)) => return Err(issues.budget.exceeded(kind)),
    }
    for file in &files {
        if !matches!(read_file(&root.join(&file.path), ReadMode::HashOnly), Ok((digest, bytes, _)) if digest == file.sha256 && bytes == file.bytes) {
            issue(issues, &file.path, IssueCause::Changed)?;
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("session_audit_response_budget")]
    ResponseLimit,
    #[error("session_audit_serialization")]
    Serialization(#[from] toml::ser::Error),
}

pub fn render(response: &AuditResponse<'_>) -> Result<String, RenderError> {
    let build = match response {
        AuditResponse::Capability { build, .. } | AuditResponse::Audit { build, .. } | AuditResponse::Incomplete { build, .. } => build,
    };
    let mut bound = Budget::new(AuditLimits::default(), build);
    bound.response(0).map_err(|_| RenderError::ResponseLimit)?;
    if let AuditResponse::Audit { files, issues, .. } = response {
        for file in files { bound.response(budget::file_bytes(&file.path)).map_err(|_| RenderError::ResponseLimit)?; }
        for issue in issues { bound.response(budget::issue_bytes(&issue.path)).map_err(|_| RenderError::ResponseLimit)?; }
    }
    // The preflight bound includes worst-case TOML escaping and all fixed fields.
    Ok(toml::to_string(response)?)
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
