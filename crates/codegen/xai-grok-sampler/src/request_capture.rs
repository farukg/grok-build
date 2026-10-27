//! Final local JSON evidence, not evidence of gateway or model consumption.

use std::fmt;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use xai_grok_sampling_types::ProjectedRequestMember;

use crate::{ApiBackend, RequestId, SamplingEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestAttemptId(pub u64);

/// Exact serializer output, before lossless HTTP content encoding. No headers are retained.
/// User content may contain secrets; this type deliberately has no Serialize implementation.
pub struct LocalRequestBody {
    bytes: Arc<[u8]>,
    digest: blake3::Hash,
}

impl fmt::Debug for LocalRequestBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalRequestBody").field("byte_len", &self.bytes.len()).finish_non_exhaustive()
    }
}

impl LocalRequestBody {
    pub(crate) fn from_serialized(bytes: Vec<u8>) -> Self {
        Self { digest: blake3::hash(&bytes), bytes: bytes.into() }
    }

    pub fn digest(&self) -> blake3::Hash { self.digest }

    /// Sensitive local-only bytes. Publication must use the scrubbed representation instead.
    pub fn sensitive_local_bytes(&self) -> &[u8] { &self.bytes }

    pub fn publication_view(&self) -> Result<PublicationBody, serde_json::Error> {
        let mut value: serde_json::Value = serde_json::from_slice(&self.bytes)?;
        let mut changed = false;
        xai_grok_secrets::walk_json_strings(&mut value, &mut |text| {
            let scrubbed = xai_grok_secrets::redact_secrets(text);
            let scrubbed = xai_grok_secrets::redact_user_paths(scrubbed.as_ref()).into_owned();
            if scrubbed != *text {
                *text = scrubbed;
                changed = true;
            }
        });
        let representation = if changed {
            PayloadRepresentation::Redacted { original_digest: *self.digest.as_bytes() }
        } else {
            PayloadRepresentation::ExactLocalJson
        };
        let bytes = if changed { serde_json::to_vec(&value)?.into() } else { Arc::clone(&self.bytes) };
        Ok(PublicationBody { bytes, representation })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "representation", rename_all = "snake_case")]
pub enum PayloadRepresentation {
    /// Unchanged by the existing scrubber; not a guarantee that arbitrary secrets can be detected.
    ExactLocalJson,
    Redacted { original_digest: [u8; 32] },
}

#[derive(Clone)]
pub struct PublicationBody {
    pub bytes: Arc<[u8]>,
    pub representation: PayloadRepresentation,
}

impl fmt::Debug for PublicationBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PublicationBody").field("byte_len", &self.bytes.len())
            .field("representation", &self.representation).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub enum LocalRequestEvidence {
    Prepared {
        backend: ApiBackend,
        body: Arc<LocalRequestBody>,
        membership: Arc<[ProjectedRequestMember]>,
    },
    /// A local HTTP execution was initiated. Delivery is not confirmed.
    DispatchStarted,
    /// HTTP response headers arrived; even a 2xx does not prove model consumption.
    ResponseHeaders { status: u16 },
    Finished { outcome: LocalAttemptOutcome },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAttemptOutcome {
    Completed,
    Empty,
    Failed,
    Cancelled,
    InitializationFailed,
}

/// One previous body per running request; the consumer owns any longer-lived retention.
#[derive(Default)]
pub(crate) struct RequestBodyCache(Mutex<Option<Arc<LocalRequestBody>>>);

#[derive(Clone)]
pub(crate) struct CaptureAttempt {
    pub request_id: RequestId,
    pub attempt: RequestAttemptId,
    pub backend: ApiBackend,
    pub membership: Arc<[ProjectedRequestMember]>,
    pub cache: Arc<RequestBodyCache>,
    pub events: mpsc::UnboundedSender<SamplingEvent>,
}

impl CaptureAttempt {
    pub fn emit(&self, evidence: LocalRequestEvidence) {
        let _ = self.events.send(SamplingEvent::LocalRequest {
            request_id: self.request_id.clone(), attempt: self.attempt, evidence,
        });
    }

    pub fn prepared(&self, bytes: &[u8]) {
        let digest = blake3::hash(bytes);
        let mut cache = match self.cache.0.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        let body = match cache.as_ref() {
            Some(previous) if previous.digest == digest && previous.bytes.as_ref() == bytes => Arc::clone(previous),
            Some(_) | None => {
                let body = Arc::new(LocalRequestBody { bytes: bytes.into(), digest });
                *cache = Some(Arc::clone(&body));
                body
            }
        };
        drop(cache);
        self.emit(LocalRequestEvidence::Prepared {
            backend: self.backend.clone(), body, membership: Arc::clone(&self.membership),
        });
    }
}
