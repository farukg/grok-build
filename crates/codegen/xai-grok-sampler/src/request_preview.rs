//! Unsent native candidate bodies; no request/attempt identity or capture events.

use xai_grok_sampling_types::{ApiBackend, ConversationRequest};
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
    if config.model.is_empty() {
        return Err(NativePreviewFailure::ModelNotInitialized);
    }
    if request.model.as_ref().is_some_and(|model| model != &config.model) {
        return Err(NativePreviewFailure::ModelConfigurationChanged);
    }
    let build = || -> xai_grok_sampling_types::Result<Vec<u8>> {
        request_body::apply_conversation_defaults(config, &mut request)?;
        match config.api_backend {
            ApiBackend::ChatCompletions => {
                let payload = request_body::apply_defaults(config, request.into())?;
                request_body::chat_body(&payload, request_body::BodyMode::Streaming)
            }
            ApiBackend::Responses => {
                let mut payload = request_body::responses_request(request, request_body::BodyMode::Streaming);
                request_body::apply_response_defaults(config, &mut payload)?;
                payload.inner.stream = Some(true);
                request_body::responses_body(config, &mut payload, request_body::BodyMode::Streaming)
            }
            ApiBackend::Messages => {
                let mut payload = request_body::messages_request(request, request_body::BodyMode::Streaming);
                request_body::apply_message_defaults(config, &mut payload)?;
                payload.inner.stream = Some(true);
                request_body::json_body(&payload.inner)
            }
        }
    };
    let bytes = build().map_err(|_| NativePreviewFailure::Serialization)?;
    Ok(NativePreview {
        backend: config.api_backend.clone(), model: config.model.clone(),
        body: LocalRequestBody::from_serialized(bytes),
    })
}
