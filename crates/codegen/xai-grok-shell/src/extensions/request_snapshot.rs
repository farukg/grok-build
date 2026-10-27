//! Passive evidence reads never enqueue prompt/model work. Resident reads share the
//! persistence resource's unbounded FIFO for consistency and can delay writes.
//! Callers must fetch bodies lazily off paint; request backlog/latency is not bounded
//! by the per-session blob caps. Cold reads activate no session or actor channel.
//! Capabilities describe main-turn capture only; absent auxiliary evidence is unknown.

use agent_client_protocol as acp;
use serde::{Deserialize, Serialize};

use super::{ExtResult, parse_params, to_raw_response};
use crate::agent::MvpAgent;
use crate::session::request_capture::CaptureFailure;
use crate::session::storage::request_capture::{SnapshotRead, SnapshotReadResult};

pub use crate::session::request_capture::{RequestSnapshotNotification, SourcePublication, SourcePublicationFailure};
pub use crate::session::request_capture::{PublicationProgress, PublicationDurability, RequestCaptureScope, UnobservedRequestPurpose};
pub use crate::session::request_capture::{REQUEST_SNAPSHOT_NOTIFICATION, SOURCE_PUBLICATION_META};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSnapshotRequest {
    pub session_id: String,
    #[serde(flatten)]
    pub action: SnapshotRead,
}

pub(crate) async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let request: RequestSnapshotRequest = parse_params(args)?;
    uuid::Uuid::try_parse(&request.session_id)
        .map_err(|_| acp::Error::invalid_params().data("sessionId must be a UUID"))?;
    if matches!(request.action, SnapshotRead::Capabilities) {
        return to_raw_response(&SnapshotReadResult::Capabilities { scope: Default::default() });
    }
    let id = acp::SessionId::new(request.session_id.clone());
    if let Some(handle) = agent.resident_handle(&id) {
        let (respond_to, response) = tokio::sync::oneshot::channel();
        if handle.persistence_tx.send(crate::session::persistence::PersistenceMsg::ReadRequestSnapshots {
            request: request.action, respond_to,
        }).is_err() {
            return to_raw_response(&SnapshotReadResult::Unavailable { cause: CaptureFailure::MailboxClosed });
        }
        let result = match response.await {
            Ok(result) => result,
            Err(_) => SnapshotReadResult::Unavailable { cause: CaptureFailure::ReplyLost },
        };
        return to_raw_response(&result);
    }
    let result = tokio::task::spawn_blocking(move || read_cold(&request.session_id, request.action)).await;
    to_raw_response(&match result {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => SnapshotReadResult::Unavailable { cause: CaptureFailure::io(&error) },
        Err(_) => SnapshotReadResult::Unavailable { cause: CaptureFailure::WorkerLost },
    })
}

fn read_cold(session_id: &str, action: SnapshotRead) -> std::io::Result<SnapshotReadResult> {
    let Some(dir) = crate::session::persistence::find_persisted_session_dir_by_id_result(session_id)? else {
        return Ok(SnapshotReadResult::Absent);
    };
    let summary: crate::session::persistence::Summary = serde_json::from_slice(
        &std::fs::read(dir.join(crate::session::storage::SUMMARY_FILE))?,
    ).map_err(std::io::Error::other)?;
    if summary.info.id.0.as_ref() != session_id {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "session identity mismatch"));
    }
    crate::session::storage::request_capture::read(&dir, session_id, action)
}
