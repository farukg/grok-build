//! Pure provider payload construction shared by preview and HTTP dispatch.

use xai_grok_sampling_types::{
    ChatCompletionRequest, ConversationRequest, CreateResponseWrapper, MessagesRequestWrapper,
    Result, SamplingError, build_messages_request,
};
use crate::config::SamplerConfig;
use crate::client::{ANTHROPIC_DEFAULT_MAX_TOKENS, StreamingChatRequest, StreamOptions,
    splice_extra_tool_entries, append_response_includes};
use xai_grok_sampling_types::rs;

#[derive(Debug, Clone, Copy)]
pub(crate) enum BodyMode { NonStreaming, Streaming }

pub(crate) fn apply_defaults(defaults: &SamplerConfig, mut request: ChatCompletionRequest) -> Result<ChatCompletionRequest> {
    if request.model.is_none() {
        request.model = Some(defaults.model.clone());
    }

    if request.max_tokens.is_none() {
        request.max_tokens = defaults.max_completion_tokens;
    }

    if request.temperature.is_none() {
        request.temperature = defaults.temperature;
    }

    if request.top_p.is_none() {
        request.top_p = defaults.top_p;
    }

    Ok(request)
}

pub(crate) fn apply_response_defaults(defaults: &SamplerConfig, request: &mut CreateResponseWrapper) -> Result<()> {
    if request.inner.model.is_none() {
        request.inner.model = Some(defaults.model.clone());
    }

    if request.inner.temperature.is_none() {
        request.inner.temperature = defaults.temperature;
    }

    if request.inner.top_p.is_none() {
        request.inner.top_p = defaults.top_p;
    }

    if request.inner.max_output_tokens.is_none() {
        request.inner.max_output_tokens = defaults.max_completion_tokens;
    }

    // The API defaults `store` to true, which breaks ZDR compliance
    if request.inner.store.is_none() {
        request.inner.store = Some(false);
    }

    if let Some(summary) = defaults.reasoning_summary {
        let summary = summary.to_responses_api();
        match request.inner.reasoning.as_mut() {
            Some(reasoning) => reasoning.summary = summary,
            None if summary.is_some() => {
                request.inner.reasoning = Some(rs::Reasoning {
                    effort: None,
                    summary,
                });
            }
            None => {}
        }
    }

    // Encrypted reasoning replay is xAI/Codex-origin only. Custom
    // endpoints (including sigma-gateway on loopback) reject unknown
    // include values; opt in via extra_response_includes instead.
    if defaults
        .extra_response_includes
        .iter()
        .any(|include| include == "reasoning.encrypted_content")
    {
        let includes = request.inner.include.get_or_insert_with(Vec::new);
        if !includes.contains(&rs::IncludeEnum::ReasoningEncryptedContent) {
            includes.push(rs::IncludeEnum::ReasoningEncryptedContent);
        }
    }

    Ok(())
}

pub(crate) fn apply_message_defaults(defaults: &SamplerConfig, request: &mut MessagesRequestWrapper) -> Result<()> {
    if request.inner.model.is_empty() {
        request.inner.model = defaults.model.clone();
    }

    if request.inner.max_tokens == 0 {
        request.inner.max_tokens = defaults
            .max_completion_tokens
            .unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS);
    }

    if request.inner.temperature.is_none() {
        request.inner.temperature = defaults.temperature;
    }

    if request.inner.top_p.is_none() {
        request.inner.top_p = defaults.top_p;
    }

    Ok(())
}

pub(crate) fn apply_conversation_defaults(defaults: &SamplerConfig, request: &mut ConversationRequest) -> Result<()> {
    if request.model.is_none() {
        request.model = Some(defaults.model.clone());
    }

    if request.temperature.is_none() {
        request.temperature = defaults.temperature;
    }

    if request.top_p.is_none() {
        request.top_p = defaults.top_p;
    }

    if request.max_output_tokens.is_none() {
        request.max_output_tokens = defaults.max_completion_tokens;
    }

    Ok(())
}

pub(crate) fn json_body<T: serde::Serialize + ?Sized>(payload: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(payload).map_err(SamplingError::Serialization)
}

pub(crate) fn chat_body(payload: &ChatCompletionRequest, mode: BodyMode) -> Result<Vec<u8>> {
    match mode {
        BodyMode::Streaming => json_body(&StreamingChatRequest {
            inner: payload, stream: true, stream_options: StreamOptions { include_usage: true },
        }),
        BodyMode::NonStreaming => json_body(payload),
    }
}

pub(crate) fn responses_body(
    defaults: &SamplerConfig, request: &mut CreateResponseWrapper, mode: BodyMode,
) -> Result<Vec<u8>> {
    let mut body = serde_json::to_value(&request.inner).map_err(SamplingError::Serialization)?;
    match mode {
        BodyMode::Streaming => {
            if defaults.stream_tool_calls && let Some(object) = body.as_object_mut() {
                object.insert("stream_tool_calls".to_owned(), serde_json::json!(true));
            }
        }
        BodyMode::NonStreaming => {}
    }
    splice_extra_tool_entries(&mut body, std::mem::take(&mut request.extra_tool_entries));
    append_response_includes(&mut body, &defaults.extra_response_includes);
    xai_grok_sampling_types::patch_reasoning_text_types(&mut body);
    json_body(&body)
}

pub(crate) fn responses_request(mut request: ConversationRequest, mode: BodyMode) -> CreateResponseWrapper {
    let trace = request.trace.take();
    let x_grok_conv_id = request.x_grok_conv_id.clone();
    let x_grok_req_id = request.x_grok_req_id.clone();
    let x_grok_session_id = request.x_grok_session_id.clone();
    let x_grok_turn_idx = request.x_grok_turn_idx.clone();
    let x_grok_transient_retry = request.x_grok_transient_retry.clone();
    let x_grok_agent_id = request.x_grok_agent_id.clone();

    // The hosted tools travel as raw JSON, spliced in after serialization by `splice_extra_tool_entries`, whose doc explains why each one does
    let extra_tools = xai_grok_sampling_types::extra_tool_entries(&request.hosted_tools);

    let responses_request: rs::CreateResponse = (&request).into();

    let mut wrapper = CreateResponseWrapper::new(responses_request);
    wrapper.x_grok_conv_id = x_grok_conv_id;
    wrapper.x_grok_req_id = x_grok_req_id;
    wrapper.x_grok_session_id = x_grok_session_id;
    wrapper.x_grok_turn_idx = x_grok_turn_idx;
    wrapper.x_grok_transient_retry = x_grok_transient_retry;
    wrapper.x_grok_agent_id = x_grok_agent_id;
    wrapper.extra_tool_entries = extra_tools;
    wrapper.traceparent = match mode {
        BodyMode::Streaming => request.traceparent,
        BodyMode::NonStreaming => None,
    };

    if let Some(trace) = trace {
        wrapper.trace = Some(trace);
    }

    wrapper
}

pub(crate) fn messages_request(mut request: ConversationRequest, mode: BodyMode) -> MessagesRequestWrapper {
    let trace = request.trace.take();
    let x_grok_conv_id = request.x_grok_conv_id.clone();
    let x_grok_req_id = request.x_grok_req_id.clone();
    let x_grok_session_id = request.x_grok_session_id.clone();
    let x_grok_turn_idx = request.x_grok_turn_idx.clone();
    let x_grok_transient_retry = request.x_grok_transient_retry.clone();
    let x_grok_agent_id = request.x_grok_agent_id.clone();

    let messages_request = build_messages_request(&request);

    let mut wrapper = MessagesRequestWrapper::new(messages_request);
    wrapper.x_grok_conv_id = x_grok_conv_id;
    wrapper.x_grok_req_id = x_grok_req_id;
    wrapper.x_grok_session_id = x_grok_session_id;
    wrapper.x_grok_turn_idx = x_grok_turn_idx;
    wrapper.x_grok_transient_retry = x_grok_transient_retry;
    wrapper.x_grok_agent_id = x_grok_agent_id;
    wrapper.traceparent = match mode {
        BodyMode::Streaming => request.traceparent,
        BodyMode::NonStreaming => None,
    };

    if let Some(trace) = trace {
        wrapper.trace = Some(trace);
    }

    wrapper
}
