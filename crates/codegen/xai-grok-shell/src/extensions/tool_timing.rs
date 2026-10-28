//! Additive ACP tool-update timing metadata; absent legacy timing is unknown.
use agent_client_protocol as acp;
pub use xai_monotonic_clock::LocalClockMapping;

pub use xai_tool_protocol::tool_execution::{
    MonotonicPoint, ToolCallOwnership, ToolCallSource, ToolExecutionId, ToolExecutionTermination,
    ToolExecutionTiming,
};

pub const TOOL_EXECUTION_TIMING_META_KEY: &str = "toolExecutionTiming";

pub fn timing_meta(timing: ToolExecutionTiming) -> Result<acp::Meta, serde_json::Error> {
    let mut meta = acp::Meta::new();
    meta.insert(
        TOOL_EXECUTION_TIMING_META_KEY.into(),
        serde_json::to_value(timing)?,
    );
    Ok(meta)
}

pub fn timing_from_meta(
    meta: Option<&acp::Meta>,
) -> Result<ToolExecutionTiming, serde_json::Error> {
    match meta.and_then(|meta| meta.get(TOOL_EXECUTION_TIMING_META_KEY)) {
        Some(value) => serde_json::from_value(value.clone()),
        None => Ok(ToolExecutionTiming::Unknown),
    }
}

pub const TOOL_CALL_OWNERSHIP_META_KEY: &str = "toolCallOwnership";

pub fn stamp_ownership(
    meta: &mut acp::Meta,
    owner: &ToolCallOwnership,
) -> Result<(), serde_json::Error> {
    meta.insert(
        TOOL_CALL_OWNERSHIP_META_KEY.into(),
        serde_json::to_value(owner)?,
    );
    Ok(())
}

pub fn stamp_original_source(meta: &mut acp::Meta, owner: &ToolCallOwnership) {
    match owner {
        ToolCallOwnership::Registered {
            source:
                ToolCallSource::Sampling {
                    prompt_id,
                    request_id,
                    attempt,
                },
            ..
        } => {
            meta.insert("promptId".into(), prompt_id.clone().into());
            meta.insert("requestId".into(), request_id.clone().into());
            meta.insert("streamEpoch".into(), (*attempt).into());
        }
        ToolCallOwnership::Registered {
            source: ToolCallSource::Unknown,
            ..
        }
        | ToolCallOwnership::Unknown => {}
    }
}

pub fn ownership_from_meta(
    meta: Option<&acp::Meta>,
) -> Result<ToolCallOwnership, serde_json::Error> {
    match meta.and_then(|meta| meta.get(TOOL_CALL_OWNERSHIP_META_KEY)) {
        Some(value) => serde_json::from_value(value.clone()),
        None => Ok(ToolCallOwnership::Unknown),
    }
}
