use std::fs::OpenOptions;
use std::io;
use std::path::{Component, Path, PathBuf};
use fs2::FileExt;
use super::{SubagentMeta, SubagentMetaStatus};
use crate::session::completion_delivery::{CompletionArtifacts, SubagentCompletionEvent};

fn with_meta_lock<T>(dir: &Path, action: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    std::fs::create_dir_all(dir)?;
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join("meta.json.lock"))?;
    lock.lock_exclusive()?;
    let result = action();
    let _ = lock.unlock();
    result
}

fn read_meta(dir: &Path) -> io::Result<SubagentMeta> {
    serde_json::from_slice(&std::fs::read(dir.join("meta.json"))?).map_err(io::Error::other)
}

fn write_meta(dir: &Path, meta: &SubagentMeta) -> io::Result<()> {
    let path = dir.join("meta.json");
    crate::session::storage::write_bytes_atomic(&path, &serde_json::to_vec_pretty(meta).map_err(io::Error::other)?)?;
    crate::session::storage::sync_parent_dir_durable(dir)?;
    if let Some(parent) = dir.parent() { crate::session::storage::sync_parent_dir_durable(parent)?; }
    Ok(())
}

pub(super) fn write_initial(dir: &Path, meta: &SubagentMeta) -> io::Result<()> {
    with_meta_lock(dir, || super::atomic_write(&dir.join("meta.json"), &serde_json::to_string_pretty(meta).map_err(io::Error::other)?))
}

pub(super) fn commit_snapshot_ref(dir: &Path, snapshot_ref: &str) -> io::Result<()> {
    with_meta_lock(dir, || {
        let mut meta = read_meta(dir)?;
        meta.snapshot_ref = Some(snapshot_ref.to_owned());
        write_meta(dir, &meta)
    })
}

fn owned_dir(parent: &Path, id: &str) -> io::Result<PathBuf> {
    let mut parts = Path::new(id).components();
    if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid child metadata identity"));
    }
    Ok(parent.join("subagents").join(id))
}

fn matches_attempt(meta: &SubagentMeta, event: &SubagentCompletionEvent, parent_session: &str) -> bool {
    meta.parent_session_id == parent_session && meta.subagent_id == event.id.subagent_id
        && meta.attempt_id.as_deref() == Some(event.id.attempt_id.as_str())
}

fn apply_outcome(meta: &mut SubagentMeta, event: &SubagentCompletionEvent) -> io::Result<()> {
    use xai_grok_tools::implementations::grok_build::task::types::SubagentState;
    if matches!(event.state, SubagentState::Running) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "nonterminal producer outcome"));
    }
    meta.status = SubagentMetaStatus::of_state(&event.state);
    meta.completed_at = Some(event.completed_at);
    meta.duration_ms = Some(event.duration_ms);
    meta.tool_calls = Some(event.tool_calls);
    meta.turns = Some(event.turns);
    meta.error = xai_grok_tools::implementations::grok_build::task::types::completion_error(&event.state);
    Ok(())
}

pub(crate) fn normalize_artifacts(parent: &Path, parent_session: &str, event: &mut SubagentCompletionEvent) -> io::Result<()> {
    let CompletionArtifacts::Metadata { metadata } = &event.artifacts else { return Ok(()) };
    if !matches_attempt(metadata, event, parent_session) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "producer metadata is not owned by this completion attempt"));
    }
    let _dir = owned_dir(parent, &event.id.subagent_id)?;
    let mut snapshot = (**metadata).clone();
    apply_outcome(&mut snapshot, event)?;
    event.artifacts = CompletionArtifacts::Metadata { metadata: Box::new(snapshot) };
    Ok(())
}

pub(crate) fn materialize_artifacts(parent: &Path, parent_session: &str, event: &SubagentCompletionEvent) -> io::Result<()> {
    match &event.artifacts {
        CompletionArtifacts::None => Ok(()),
        CompletionArtifacts::Metadata { metadata } => {
            if !matches_attempt(metadata, event, parent_session) {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "completion metadata scope mismatch"));
            }
            let dir = owned_dir(parent, &event.id.subagent_id)?;
            with_meta_lock(&dir, || {
                match read_meta(&dir) {
                    Ok(current) if !matches_attempt(&current, event, parent_session) => return Ok(()),
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
                write_meta(&dir, metadata)
            })
        }
    }
}

#[cfg(test)]
pub(super) fn materialize_at(dir: &Path, event: &SubagentCompletionEvent) -> io::Result<()> {
    match &event.artifacts {
        CompletionArtifacts::None => Ok(()),
        CompletionArtifacts::Metadata { metadata } => with_meta_lock(dir, || write_meta(dir, metadata)),
    }
}
