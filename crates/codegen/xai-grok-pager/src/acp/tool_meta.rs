use agent_client_protocol as acp;
use uuid::Uuid;
use xai_grok_shell::extensions::tool_timing::{
    ToolCallOwnership, ToolCallSource, ToolExecutionTiming, ownership_from_meta, timing_from_meta,
};
use super::meta::NotificationMeta;
use xai_grok_shell::extensions::sampling_lifecycle::{SamplingOwner, RequestAttemptId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolSource {
    Unknown,
    Sampling(SamplingOwner),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolIdentity {
    pub(crate) invocation: Uuid,
    pub(crate) provider: String,
    pub(crate) source: ToolSource,
}

impl ToolIdentity {
    pub(crate) fn from_ownership(ownership: &ToolCallOwnership) -> Option<Self> {
        let ToolCallOwnership::Registered { invocation_id, provider_call_id, source } = ownership else { return None; };
        let invocation = Uuid::parse_str(invocation_id).ok().filter(|id| !id.is_nil())?;
        if provider_call_id.is_empty() { return None; }
        let source = match source {
            ToolCallSource::Unknown => ToolSource::Unknown,
            ToolCallSource::Sampling { prompt_id, request_id, attempt } => {
                if prompt_id.is_empty() || request_id.is_empty() || *attempt == 0 { return None; }
                ToolSource::Sampling(SamplingOwner { prompt_id: prompt_id.clone(), request_id: request_id.clone().into(), attempt: RequestAttemptId(*attempt) })
            }
        };
        Some(Self { invocation, provider: provider_call_id.clone(), source })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) enum ToolMetadata {
    #[default]
    NotTool,
    Legacy,
    Owned { identity: ToolIdentity, timing: ToolExecutionTiming },
    Malformed,
}

impl ToolMetadata {
    pub(crate) fn parse(update: &acp::SessionUpdate, outer: &NotificationMeta) -> Self {
        let (provider, meta) = match update {
            acp::SessionUpdate::ToolCall(call) => (call.tool_call_id.0.as_ref(), call.meta.as_ref()),
            acp::SessionUpdate::ToolCallUpdate(call) => (call.tool_call_id.0.as_ref(), call.meta.as_ref()),
            _ => return Self::NotTool,
        };
        let parse = || -> Option<Self> {
            let timing = timing_from_meta(meta).ok()?;
            if let Some(id) = meta.and_then(|meta| meta.get("toolExecutionTiming"))
                .and_then(|timing| timing.get("executionId"))
            {
                if Uuid::parse_str(id.as_str()?).ok()?.is_nil() { return None; }
            }
            let ownership = ownership_from_meta(meta).ok()?;
            match &ownership {
                ToolCallOwnership::Unknown => Some(Self::Legacy),
                ToolCallOwnership::Registered { .. } => {
                    let identity = ToolIdentity::from_ownership(&ownership)?;
                    if identity.provider != provider { return None; }
                    if let ToolSource::Sampling(owner) = &identity.source {
                        if outer.prompt_id.as_ref().is_some_and(|id| id != &owner.prompt_id)
                            || outer.xai_request_id.as_ref().is_some_and(|id| id != owner.request_id.as_str())
                            || outer.xai_stream_epoch.is_some_and(|attempt| attempt != owner.attempt.0)
                            || outer.sampling_owner().is_some_and(|outer| outer != owner) { return None; }
                    }
                    Some(Self::Owned { identity, timing })
                }
            }
        };
        parse().unwrap_or(Self::Malformed)
    }
}
