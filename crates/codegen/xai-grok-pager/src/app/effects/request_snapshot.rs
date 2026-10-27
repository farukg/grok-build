//! Passive capture reads for the request inspector; no model work or session activation.

use agent_client_protocol as acp;
use xai_acp_lib::acp_send;
use xai_grok_shell::extensions::request_snapshot::RequestSnapshotRequest;
use xai_grok_shell::session::request_capture::{BodyAvailability, CaptureFailure, RequestBodyRead};
use xai_grok_shell::session::storage::request_capture::{
    BodySelection, SnapshotRead, SnapshotReadResult,
};

use crate::app::actions::{RequestSnapshotQuery, RequestSnapshotTransportError, TaskResult};
use crate::app::agent::AgentId;
use crate::views::request_viewer::RequestBodyContent;

async fn send(
    session_id: &acp::SessionId,
    action: SnapshotRead,
    tx: &xai_acp_lib::AcpAgentTx,
) -> Result<SnapshotReadResult, RequestSnapshotTransportError> {
    let request = RequestSnapshotRequest {
        session_id: session_id.0.to_string(),
        action,
    };
    let raw =
        serde_json::value::to_raw_value(&request).map_err(RequestSnapshotTransportError::Encode)?;
    let response = acp_send(
        acp::ExtRequest::new("x.ai/session/request_snapshot", raw.into()),
        tx,
    )
    .await
    .map_err(RequestSnapshotTransportError::Rpc)?;
    serde_json::from_str::<SnapshotReadResult>(response.0.get())
        .map_err(RequestSnapshotTransportError::Decode)
}

pub(super) async fn query(
    agent_id: AgentId,
    session_id: acp::SessionId,
    query: RequestSnapshotQuery,
    generation: u64,
    tx: &xai_acp_lib::AcpAgentTx,
) -> TaskResult {
    match query {
        RequestSnapshotQuery::Latest => {
            let result = match send(&session_id, SnapshotRead::Latest, tx).await {
                Ok(SnapshotReadResult::Snapshot { snapshot }) => {
                    if snapshot.origin.session_id == session_id.0.as_ref() {
                        Ok(Some(snapshot))
                    } else {
                        Err(RequestSnapshotTransportError::UnexpectedShape)
                    }
                }
                Ok(SnapshotReadResult::Absent) => Ok(None),
                Ok(SnapshotReadResult::Unavailable { cause }) =>
                    Err(RequestSnapshotTransportError::Unavailable(cause)),
                Ok(SnapshotReadResult::Capabilities { .. }
                | SnapshotReadResult::List { .. }
                | SnapshotReadResult::Body { .. }) =>
                    Err(RequestSnapshotTransportError::UnexpectedShape),
                Err(error) => Err(error),
            };
            TaskResult::RequestSnapshotLoaded { agent_id, session_id, generation, result }
        }
        RequestSnapshotQuery::Body { key } => {
            let result = match send(
                &session_id,
                SnapshotRead::Body {
                    request: key.0.snapshot.request,
                    attempt: key.0.attempt,
                    selection: BodySelection::Digest { digest: key.0.digest },
                },
                tx,
            ).await {
                Ok(SnapshotReadResult::Body { body }) => Ok(body),
                Ok(SnapshotReadResult::Absent) => Ok(RequestBodyRead::Unavailable {
                    availability: BodyAvailability::Unknown,
                }),
                Ok(SnapshotReadResult::Unavailable { cause }) =>
                    Err(RequestSnapshotTransportError::Unavailable(cause)),
                Ok(SnapshotReadResult::Capabilities { .. }
                | SnapshotReadResult::List { .. }
                | SnapshotReadResult::Snapshot { .. }) =>
                    Err(RequestSnapshotTransportError::UnexpectedShape),
                Err(error) => Err(error),
            };
            let result = match result {
                Ok(body) => {
                    let selected = key.clone();
                    tokio::task::spawn_blocking(move || RequestBodyContent::prepare(&selected, body))
                        .await
                        .map_err(|_| RequestSnapshotTransportError::Unavailable(CaptureFailure::WorkerLost))
                        .and_then(|result| result)
                }
                Err(error) => Err(error),
            };
            TaskResult::RequestBodyLoaded { agent_id, session_id, generation, key, result }
        }
    }
}

#[cfg(test)]
#[path = "request_snapshot_tests.rs"]
mod tests;
