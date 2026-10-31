//! Unsent native candidate bodies; no request/attempt identity or capture events.

use xai_grok_sampling_types::{ApiBackend, ConversationRequest};
use crate::client::ClientDefaults;
use crate::config::SamplerConfig;
use crate::request_body;
use crate::request_capture::LocalRequestBody;

#[derive(Debug)]
pub struct NativePreview {
    pub backend: ApiBackend,
    pub model: String,
    pub body: LocalRequestBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePreviewFailure {
    MailboxClosed,
    ReplyLost,
    ModelNotInitialized,
    ModelConfigurationChanged,
    Serialization,
}

pub(crate) fn preview(
    config: &SamplerConfig, mut request: ConversationRequest,
) -> Result<NativePreview, NativePreviewFailure> {
    let defaults = ClientDefaults::from(config);
    if defaults.model.is_empty() {
        return Err(NativePreviewFailure::ModelNotInitialized);
    }
    if request.model.as_ref().is_some_and(|model| model != &defaults.model) {
        return Err(NativePreviewFailure::ModelConfigurationChanged);
    }
    let build = || -> xai_grok_sampling_types::Result<Vec<u8>> {
        request_body::apply_conversation_defaults(&defaults, &mut request)?;
        match defaults.api_backend {
            ApiBackend::ChatCompletions => {
                let payload = request_body::apply_defaults(&defaults, request.into())?;
                request_body::chat_body(&payload, request_body::BodyMode::Streaming)
            }
            ApiBackend::Responses => {
                let (mut payload, _) = request_body::responses_request(request, request_body::BodyMode::Streaming);
                request_body::apply_response_defaults(&defaults, &mut payload)?;
                payload.inner.stream = Some(true);
                request_body::responses_body(&defaults, &mut payload, request_body::BodyMode::Streaming)
            }
            ApiBackend::Messages => {
                let (mut payload, _) = request_body::messages_request(request, request_body::BodyMode::Streaming);
                request_body::apply_message_defaults(&defaults, &mut payload)?;
                payload.inner.stream = Some(true);
                request_body::json_body(&payload.inner)
            }
        }
    };
    let bytes = build().map_err(|_| NativePreviewFailure::Serialization)?;
    Ok(NativePreview {
        backend: defaults.api_backend.clone(), model: defaults.model.clone(),
        body: LocalRequestBody::from_serialized(bytes),
    })
}
