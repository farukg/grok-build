use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use xai_grok_sampler::request_capture::{LocalRequestBody, LocalRequestEvidence, RequestAttemptId};

pub use xai_grok_sampler::request_capture::PayloadRepresentation;

use crate::session::request_capture::*;

pub(crate) const DIRECTORY: &str = "request-capture-v1";
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CACHE_BLOBS: usize = 8;
/// Tail-read granularity for the request index; bounds I/O per step, not history length.
const INDEX_TAIL_CHUNK_BYTES: u64 = 512;

#[derive(Debug, Serialize, Deserialize, Default)]
struct BodyCache { entries: Vec<CacheEntry> }

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    digest: [u8; 32],
    bytes: u64,
    byte_len: u64,
    publication_digest: [u8; 32],
    representation: xai_grok_sampler::request_capture::PayloadRepresentation,
    state: CacheState,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CacheState { Writing, Ready }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotPage {
    pub requests: Vec<RequestSnapshotRef>,
    pub next: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SnapshotRead {
    Capabilities,
    List { #[serde(default)] after: u64, #[serde(default = "default_page_limit")] limit: u16 },
    Get { request: RequestSnapshotId },
    Body { request: RequestSnapshotId, attempt: RequestAttemptId, #[serde(default)] selection: BodySelection },
    Latest,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BodySelection {
    #[default]
    LatestPrepared,
    Digest { digest: [u8; 32] },
}

fn default_page_limit() -> u16 { 50 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum SnapshotReadResult {
    Capabilities { scope: RequestCaptureScope },
    List { page: SnapshotPage },
    Snapshot { snapshot: RequestSnapshot },
    Body { body: RequestBodyRead },
    Absent,
    Unavailable { cause: CaptureFailure },
}

pub(crate) fn read(session_dir: &Path, session_id: &str, request: SnapshotRead) -> io::Result<SnapshotReadResult> {
    let root = session_dir.join(DIRECTORY);
    if !matches!(request, SnapshotRead::Capabilities) {
        if !root.exists() { return Ok(SnapshotReadResult::Absent); }
        require_directory(&root)?;
    }
    match request {
        SnapshotRead::Capabilities => Ok(SnapshotReadResult::Capabilities { scope: Default::default() }),
        SnapshotRead::List { after, limit } => {
            let limit = usize::from(limit.clamp(1, 100));
            let file = match regular_file(&root.join("requests.jsonl")) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(SnapshotReadResult::Absent),
                Err(error) => return Err(error),
            };
            let mut lines = io::BufReader::new(file).lines().skip(usize::try_from(after).map_err(io::Error::other)?);
            let mut requests = Vec::with_capacity(limit);
            for _ in 0..limit {
                let Some(line) = lines.next() else { break; };
                let id = serde_json::from_str::<RequestSnapshotId>(&line?).map_err(io::Error::other)?;
                requests.push(RequestSnapshotRef { session_id: session_id.to_owned(), request: id });
            }
            let next = if requests.len() == limit { Some(after.saturating_add(requests.len() as u64)) } else { None };
            Ok(SnapshotReadResult::List { page: SnapshotPage { requests, next } })
        }
        SnapshotRead::Get { request } => resolved_snapshot(&root, session_id, request),
        SnapshotRead::Latest => {
            let file = match regular_file(&root.join("requests.jsonl")) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(SnapshotReadResult::Absent),
                Err(error) => return Err(error),
            };
            let Some(request) = latest_index_request(file)? else { return Ok(SnapshotReadResult::Absent); };
            resolved_snapshot(&root, session_id, request)
        }
        SnapshotRead::Body { request, attempt, selection } => {
            let Some(snapshot) = load_snapshot(&root, session_id, request)? else { return Ok(SnapshotReadResult::Absent); };
            let body = snapshot.observations.iter().rev().find_map(|entry| {
                if entry.attempt != attempt { return None; }
                match &entry.observation {
                    RequestObservation::Prepared { body, .. } => match selection {
                        BodySelection::LatestPrepared => Some(body.clone()),
                        BodySelection::Digest { digest } => (body.digest == digest).then(|| body.clone()),
                    },
                    RequestObservation::DispatchStarted | RequestObservation::ResponseHeaders { .. } | RequestObservation::Finished { .. } => None,
                }
            });
            let Some(mut prepared) = body else {
                return Ok(SnapshotReadResult::Body { body: RequestBodyRead::Unavailable { availability: BodyAvailability::Unknown } });
            };
            resolve_availability(&root, &load_cache(&root)?, &mut prepared);
            let body = match prepared.availability {
                BodyAvailability::Captured { publication_digest, representation } => {
                    let digest = prepared.digest;
                    let raw = read_raw(&root, digest)?;
                    if raw.len() as u64 != prepared.byte_len { return Err(invalid()); }
                    let json = read_view(&root, digest)?;
                    if blake3::hash(json.as_bytes()).as_bytes() != &publication_digest { return Err(invalid()); }
                    RequestBodyRead::Available {
                        json,
                        representation,
                        protected_local: ProtectedBodyRef {
                            snapshot: RequestSnapshotRef { session_id: session_id.to_owned(), request }, attempt, digest,
                        },
                    }
                }
                availability @ (BodyAvailability::Expired | BodyAvailability::NotCaptured { .. }
                    | BodyAvailability::Failed { .. } | BodyAvailability::Unknown) => RequestBodyRead::Unavailable { availability },
            };
            Ok(SnapshotReadResult::Body { body })
        }
    }
}

fn resolved_snapshot(root: &Path, session_id: &str, request: RequestSnapshotId) -> io::Result<SnapshotReadResult> {
    let Some(mut snapshot) = load_snapshot(root, session_id, request)? else { return Ok(SnapshotReadResult::Absent); };
    let cache = load_cache(root)?;
    for observation in &mut snapshot.observations {
        if let RequestObservation::Prepared { body, .. } = &mut observation.observation {
            resolve_availability(root, &cache, body);
        }
    }
    Ok(SnapshotReadResult::Snapshot { snapshot })
}

/// Newest index entry, read from the tail of `requests.jsonl` so any history
/// size resolves without full prefix pages. A malformed final line keeps its
/// error; an older entry is never served as a hidden substitute.
fn latest_index_request(mut file: File) -> io::Result<Option<RequestSnapshotId>> {
    use std::io::{Read, Seek, SeekFrom};
    let len = file.metadata()?.len();
    if len == 0 { return Ok(None); }
    let mut end = len;
    {
        let mut final_byte = [0; 1];
        file.seek(SeekFrom::Start(len - 1))?;
        file.read_exact(&mut final_byte)?;
        if final_byte[0] == b'\n' { end -= 1; }
    }
    let mut chunks = Vec::new();
    let mut position = end;
    while position > 0 {
        let start = position.saturating_sub(INDEX_TAIL_CHUNK_BYTES);
        let mut chunk = vec![0; (position - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk)?;
        match chunk.iter().rposition(|&byte| byte == b'\n') {
            Some(offset) => {
                chunks.push(chunk.into_iter().skip(offset + 1).collect());
                break;
            }
            None => {
                chunks.push(chunk);
                position = start;
            }
        }
    }
    chunks.reverse();
    let line = chunks.into_iter().flatten().collect::<Vec<u8>>();
    let line = String::from_utf8(line).map_err(|_| invalid())?;
    serde_json::from_str::<RequestSnapshotId>(&line).map_err(io::Error::other).map(Some)
}

pub(crate) fn published_sources(session_dir: &Path) -> io::Result<Vec<xai_grok_sampling_types::SourceOccurrenceId>> {
    let root = session_dir.join(DIRECTORY);
    if root.try_exists()? { require_directory(&root)?; }
    match regular_file(&root.join("source-publications.jsonl")) {
        Ok(file) => {
            for line in io::BufReader::new(file).lines() {
                let _: xai_grok_sampling_types::SourceOccurrenceId = serde_json::from_str(&line?).map_err(|_| invalid())?;
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    // The ledger is a cache; only complete typed journal records prove application.
    let updates = match regular_file(&session_dir.join(super::UPDATES_FILE)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut reader = io::BufReader::new(updates);
    let mut line = Vec::new();
    let mut ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 { break; }
        if !line.ends_with(b"\n") { break; }
        let record = &line[..line.len() - 1];
        let text = match std::str::from_utf8(record) {
            Ok(text) => text,
            Err(error) if error.error_len().is_none() => continue,
            Err(_) => return Err(invalid()),
        };
        let update = match super::SessionUpdateEnvelope::from_str(text) {
            Ok(update) => update,
            // The append owner terminates a crashed partial record before the next append.
            Err(error) if error.is_eof() => continue,
            Err(_) => return Err(invalid()),
        };
        if let Some(id) = publication_source_id(&update)? {
            if seen.insert(id) { ids.push(id); }
        }
    }
    Ok(ids)
}

pub(crate) fn publication_source_id(update: &super::SessionUpdate) -> io::Result<Option<xai_grok_sampling_types::SourceOccurrenceId>> {
    match update {
        super::SessionUpdate::Acp(_) => Ok(None),
        super::SessionUpdate::Xai(notification) => {
            if !matches!(&notification.update, crate::extensions::notification::SessionUpdate::InjectedContextRecord { .. }) { return Ok(None); }
            let Some(metadata) = notification.meta.as_ref().and_then(|meta| meta.get(SOURCE_PUBLICATION_META)) else { return Ok(None); };
            let source: SourcePublication = serde_json::from_value(metadata.clone()).map_err(|_| invalid())?;
            match source.source {
                xai_grok_sampling_types::SourceOccurrence::Admitted { id } => Ok(Some(id)),
                xai_grok_sampling_types::SourceOccurrence::LegacyNormalized { .. } | xai_grok_sampling_types::SourceOccurrence::Unadmitted => Ok(None),
            }
        }
    }
}

pub(crate) fn claim_source_publications(session_dir: &Path, sources: &[xai_grok_sampling_types::SourceOccurrenceId]) -> io::Result<()> {
    let mut ids = sources.to_vec();
    if ids.is_empty() { return Ok(()); }
    let root = prepare_root(session_dir)?;
    let _lock = lock(&root)?;
    let path = root.join("source-publications.jsonl");
    if !path.try_exists()? {
        let mut restored = published_sources(session_dir)?;
        restored.extend(ids);
        ids = restored;
    }
    let mut file = private_append(&path)?;
    let mut bytes = Vec::new();
    for id in ids {
        serde_json::to_writer(&mut bytes, &id).map_err(io::Error::other)?;
        bytes.push(b'\n');
    }
    file.write_all(&bytes)?;
    super::sync_file_durable(&file)?;
    super::sync_dir_durable(&root)
}

pub(crate) fn begin(session_dir: &Path, origin: &RequestOrigin) -> io::Result<()> {
    let root = prepare_root(session_dir)?;
    let _lock = lock(&root)?;
    if let Some(previous) = load_snapshot(&root, &origin.session_id, origin.request)? {
        return if previous.origin == *origin { Ok(()) } else { Err(invalid()) };
    }
    let mut index = private_append(&root.join("requests.jsonl"))?;
    let mut line = serde_json::to_vec(&origin.request).map_err(io::Error::other)?;
    line.push(b'\n');
    index.write_all(&line)?;
    super::sync_file_durable(&index)?;
    super::sync_dir_durable(&root)?;
    save_json(&snapshot_path(&root, origin.request), &RequestSnapshot { origin: origin.clone(), observations: Vec::new() })
}

pub(crate) fn record(
    session_dir: &Path,
    session_id: &str,
    request: RequestSnapshotId,
    attempt: RequestAttemptId,
    evidence: LocalRequestEvidence,
) -> RequestSnapshotNotification {
    let snapshot_ref = RequestSnapshotRef { session_id: session_id.to_owned(), request };
    let root = session_dir.join(DIRECTORY);
    let observation = observation(&evidence);
    let mut notification = RequestSnapshotNotification {
        snapshot: snapshot_ref,
        evidence: AttemptObservation { attempt, observation },
        receipt: CaptureReceipt::Unconfirmed { cause: CaptureFailure::Storage },
    };
    let apply = || -> io::Result<RequestSnapshotNotification> {
        require_directory(&root)?;
        let _lock = lock(&root)?;
        let Some(mut snapshot) = load_snapshot(&root, session_id, request)? else {
            let mut notification = notification.clone();
            notification.receipt = CaptureReceipt::Unconfirmed { cause: CaptureFailure::OriginUnavailable };
            return Ok(notification);
        };
        let mut notification = notification.clone();
        if let (LocalRequestEvidence::Prepared { body, .. }, RequestObservation::Prepared { body: availability, .. }) = (&evidence, &mut notification.evidence.observation) {
            availability.availability = match retain_body(&root, body) {
                Ok(availability) => availability,
                Err(error) => BodyAvailability::Failed { cause: CaptureFailure::io(&error) },
            };
        }
        snapshot.observations.push(notification.evidence.clone());
        save_json(&snapshot_path(&root, request), &snapshot)?;
        notification.receipt = CaptureReceipt::Confirmed { origin: snapshot.origin };
        Ok(notification)
    };
    match apply() {
        Ok(notification) => notification,
        Err(error) => {
            notification.receipt = CaptureReceipt::Unconfirmed { cause: CaptureFailure::io(&error) };
            if let RequestObservation::Prepared { body, .. } = &mut notification.evidence.observation {
                body.availability = BodyAvailability::Failed { cause: CaptureFailure::io(&error) };
            }
            notification
        }
    }
}

pub(crate) fn observation(evidence: &LocalRequestEvidence) -> RequestObservation {
    match evidence {
        LocalRequestEvidence::Prepared { backend, membership, body } => RequestObservation::Prepared {
            backend: backend.clone(), membership: membership.to_vec(), body: PreparedBody {
                digest: *body.digest().as_bytes(), byte_len: body.sensitive_local_bytes().len() as u64,
                availability: BodyAvailability::Unknown,
            },
            source_durability: SourceDurability::UnconfirmedIndependentHistoryChannel,
        },
        LocalRequestEvidence::DispatchStarted => RequestObservation::DispatchStarted,
        LocalRequestEvidence::ResponseHeaders { status } => RequestObservation::ResponseHeaders { status: *status },
        LocalRequestEvidence::Finished { outcome } => RequestObservation::Finished { outcome: (*outcome).into() },
    }
}

fn retain_body(root: &Path, body: &LocalRequestBody) -> io::Result<BodyAvailability> {
    let byte_len = body.sensitive_local_bytes().len() as u64;
    if byte_len > MAX_BODY_BYTES {
        return Ok(BodyAvailability::NotCaptured { reason: NotCapturedReason::Oversize });
    }
    let digest = *body.digest().as_bytes();
    let mut cache = load_cache(root)?;
    if let Some(position) = cache.entries.iter().position(|entry| entry.digest == digest) {
        match cache.entries[position].state {
            CacheState::Writing => {
                // The exclusive capture lock proves the old writer no longer owns this intent.
                remove_blob(root, digest, "raw")?;
                remove_blob(root, digest, "view")?;
                cache.entries.remove(position);
                save_json(&root.join("bodies.json"), &cache)?;
            }
            CacheState::Ready => {
                let entry = cache.entries.remove(position);
                if entry.byte_len != byte_len
                    || !blob_path(root, digest, "raw").is_file() || !blob_path(root, digest, "view").is_file() {
                    return Err(invalid());
                }
                if read_raw(root, digest)?.as_slice() != body.sensitive_local_bytes() { return Err(invalid()); }
                let availability = BodyAvailability::Captured {
                    publication_digest: entry.publication_digest, representation: entry.representation,
                };
                cache.entries.push(entry);
                save_json(&root.join("bodies.json"), &cache)?;
                return Ok(availability);
            }
        }
    }
    let view = body.publication_view().map_err(io::Error::other)?;
    let publication_digest = *blake3::hash(&view.bytes).as_bytes();
    let bytes = byte_len.saturating_add(view.bytes.len() as u64);
    if bytes > MAX_CACHE_BYTES {
        return Ok(BodyAvailability::NotCaptured { reason: NotCapturedReason::Oversize });
    }
    while cache.entries.len() >= MAX_CACHE_BLOBS || cache.entries.iter().fold(0u64, |total, entry| total.saturating_add(entry.bytes)).saturating_add(bytes) > MAX_CACHE_BYTES {
        let Some(oldest) = cache.entries.first() else { return Err(invalid()); };
        remove_blob(root, oldest.digest, "raw")?;
        remove_blob(root, oldest.digest, "view")?;
        cache.entries.remove(0);
        save_json(&root.join("bodies.json"), &cache)?;
    }
    let raw_path = blob_path(root, digest, "raw");
    let view_path = blob_path(root, digest, "view");
    require_absent(&raw_path)?;
    require_absent(&view_path)?;
    cache.entries.push(CacheEntry { digest, bytes, byte_len, publication_digest, representation: view.representation, state: CacheState::Writing });
    save_json(&root.join("bodies.json"), &cache)?;
    save_text(&raw_path, std::str::from_utf8(body.sensitive_local_bytes()).map_err(io::Error::other)?)?;
    save_text(&view_path, std::str::from_utf8(&view.bytes).map_err(io::Error::other)?)?;
    let Some(entry) = cache.entries.last_mut() else { return Err(invalid()); };
    entry.state = CacheState::Ready;
    save_json(&root.join("bodies.json"), &cache)?;
    Ok(BodyAvailability::Captured { publication_digest, representation: view.representation })
}

fn resolve_availability(root: &Path, cache: &BodyCache, body: &mut PreparedBody) {
    if matches!(body.availability, BodyAvailability::Captured { .. }) {
        if !root.join("bodies.json").is_file() {
            body.availability = BodyAvailability::Unknown;
            return;
        }
        match cache.entries.iter().find(|entry| entry.digest == body.digest) {
            None => body.availability = BodyAvailability::Expired,
            Some(entry) => match entry.state {
                CacheState::Writing => body.availability = BodyAvailability::Unknown,
                CacheState::Ready => {
                    if !blob_path(root, body.digest, "raw").is_file() || !blob_path(root, body.digest, "view").is_file() {
                        body.availability = BodyAvailability::Unknown;
                    }
                }
            },
        }
    }
}

fn load_snapshot(root: &Path, session_id: &str, request: RequestSnapshotId) -> io::Result<Option<RequestSnapshot>> {
    let file = match regular_file(&snapshot_path(root, request)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let snapshot: RequestSnapshot = serde_json::from_reader(file).map_err(io::Error::other)?;
    if snapshot.origin.session_id != session_id || snapshot.origin.request != request { return Err(invalid()); }
    Ok(Some(snapshot))
}

fn load_cache(root: &Path) -> io::Result<BodyCache> {
    match regular_file(&root.join("bodies.json")) {
        Ok(file) => serde_json::from_reader(file).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BodyCache::default()),
        Err(error) => Err(error),
    }
}

fn read_raw(root: &Path, digest: [u8; 32]) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    regular_file(&blob_path(root, digest, "raw"))?.take(MAX_BODY_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BODY_BYTES || blake3::hash(&bytes).as_bytes() != &digest { return Err(invalid()); }
    Ok(bytes)
}

fn read_view(root: &Path, digest: [u8; 32]) -> io::Result<String> {
    use std::io::Read;
    let mut text = String::new();
    regular_file(&blob_path(root, digest, "view"))?.take(MAX_CACHE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_CACHE_BYTES { return Err(invalid()); }
    let _: serde_json::Value = serde_json::from_str(&text).map_err(io::Error::other)?;
    Ok(text)
}

fn snapshot_path(root: &Path, request: RequestSnapshotId) -> PathBuf { root.join(format!("{}.json", request.0)) }

fn blob_path(root: &Path, digest: [u8; 32], kind: &str) -> PathBuf {
    root.join(format!("{}.{}.json", blake3::Hash::from_bytes(digest).to_hex(), kind))
}

fn remove_blob(root: &Path, digest: [u8; 32], kind: &str) -> io::Result<()> {
    let path = blob_path(root, digest, kind);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() => std::fs::remove_file(path),
        Ok(_) => Err(invalid()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn prepare_root(session_dir: &Path) -> io::Result<PathBuf> {
    require_directory(session_dir)?;
    let root = session_dir.join(DIRECTORY);
    if root.try_exists()? { require_directory(&root)?; }
    super::create_dir_all_durable_with(&root, crate::util::grok_home::create_dir_all_owner_only, super::sync_dir_durable)?;
    Ok(root)
}

fn require_directory(path: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(path)?.file_type().is_dir() { Ok(()) } else { Err(invalid()) }
}

fn regular_file(path: &Path) -> io::Result<File> {
    if !std::fs::symlink_metadata(path)?.file_type().is_file() { return Err(invalid()); }
    File::open(path)
}

fn require_absent(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(io::Error::new(io::ErrorKind::AlreadyExists, "local capture destination already exists")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn private_append(path: &Path) -> io::Result<File> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Err(invalid()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true).read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn lock(root: &Path) -> io::Result<File> {
    let file = private_append(&root.join("capture.lock"))?;
    file.try_lock_exclusive()?;
    Ok(file)
}

fn save_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    save_text(path, &serde_json::to_string(value).map_err(io::Error::other)?)
}

fn save_text(path: &Path, text: &str) -> io::Result<()> {
    super::write_private_bytes_atomic(path, text.as_bytes())
}

fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "invalid local request capture") }

#[cfg(test)]
#[path = "request_capture_tests.rs"]
pub(crate) mod tests;
