use super::SessionPersistence;
use crate::session::request_capture::*;
use crate::session::storage::request_capture::{self, SnapshotRead, SnapshotReadResult};

impl SessionPersistence {
    fn request_capture_dir(&self) -> Option<std::path::PathBuf> {
        self.storage.updates_file_path(&self.info)?.parent().map(std::path::Path::to_path_buf)
    }

    pub(super) async fn read_published_request_sources(&self) -> std::io::Result<Vec<xai_grok_sampling_types::SourceOccurrenceId>> {
        let dir = self.request_capture_dir().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "session capture directory unavailable"))?;
        self.storage.sync_session_files_selected(&self.info, crate::session::storage::SessionFileSet {
            updates: true, ..Default::default()
        }).await?;
        tokio::task::spawn_blocking(move || request_capture::published_sources(&dir)).await.map_err(std::io::Error::other)?
    }

    pub(super) async fn claim_request_source_publications(&self, updates: &[crate::session::storage::SessionUpdate]) -> std::io::Result<()> {
        let dir = self.request_capture_dir().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "session capture directory unavailable"))?;
        let mut ids = Vec::new();
        for update in updates {
            if let Some(id) = request_capture::publication_source_id(update)? { ids.push(id); }
        }
        tokio::task::spawn_blocking(move || request_capture::claim_source_publications(&dir, &ids)).await.map_err(std::io::Error::other)?
    }

    pub(super) async fn begin_request_capture(&self, origin: RequestOrigin) {
        if origin.session_id != self.info.id.0.as_ref() { return; }
        let Some(dir) = self.request_capture_dir() else { return; };
        let _ = tokio::task::spawn_blocking(move || request_capture::begin(&dir, &origin)).await;
    }

    pub(super) async fn record_request_capture(
        &self,
        request: RequestSnapshotId,
        attempt: xai_grok_sampler::request_capture::RequestAttemptId,
        evidence: xai_grok_sampler::request_capture::LocalRequestEvidence,
    ) -> RequestSnapshotNotification {
        let fallback = RequestSnapshotNotification {
            snapshot: RequestSnapshotRef { session_id: self.info.id.to_string(), request },
            evidence: AttemptObservation { attempt, observation: request_capture::observation(&evidence) },
            receipt: CaptureReceipt::Unconfirmed { cause: CaptureFailure::Storage },
        };
        let Some(dir) = self.request_capture_dir() else { return fallback; };
        let session_id = self.info.id.to_string();
        match tokio::task::spawn_blocking(move || request_capture::record(&dir, &session_id, request, attempt, evidence)).await {
            Ok(notification) => notification,
            Err(_) => RequestSnapshotNotification {
                receipt: CaptureReceipt::Unconfirmed { cause: CaptureFailure::WorkerLost }, ..fallback
            },
        }
    }

    pub(super) async fn read_request_snapshots(&self, request: SnapshotRead) -> SnapshotReadResult {
        let Some(dir) = self.request_capture_dir() else {
            return SnapshotReadResult::Unavailable { cause: CaptureFailure::Storage };
        };
        let session_id = self.info.id.to_string();
        match tokio::task::spawn_blocking(move || request_capture::read(&dir, &session_id, request)).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => SnapshotReadResult::Unavailable { cause: CaptureFailure::io(&error) },
            Err(_) => SnapshotReadResult::Unavailable { cause: CaptureFailure::WorkerLost },
        }
    }
}

#[cfg(test)]
#[path = "persistence_request_capture_tests.rs"]
mod tests;
