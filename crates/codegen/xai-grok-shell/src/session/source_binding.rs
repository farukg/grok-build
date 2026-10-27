use serde::{Deserialize, Serialize};
use super::request_capture::{InputSourceBinding, InputSourceNotification};

#[cfg(test)]
#[path = "source_binding_tests.rs"]
mod tests;

pub const SOURCE_BINDING_NOTIFICATION: &str = "x.ai/session/request_source/observed";
pub const OUTPUT_ORIGIN_META: &str = "requestOutput";
pub const STRICT_ORIGIN_META: &str = "requestStrict";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum StrictSourceOrigin {
    Completion { session_id: String, completions: Vec<xai_chat_state::completion::CompletionIdentity> },
    CompletionToolResult { session_id: String, completions: Vec<xai_chat_state::completion::CompletionIdentity>, tool_call_id: String },
    Notification { session_id: String, batch_id: String, completions: Vec<xai_chat_state::completion::CompletionIdentity> },
    Delivery { session_id: String, delivery_id: String },
    ToolResult { session_id: String, tool_call_id: String },
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum StrictInputRef {
    CompletionAggregate { session_id: String, completions: Vec<xai_chat_state::completion::CompletionIdentity> },
    Notification { session_id: String, batch_id: String },
    Delivery { session_id: String, delivery_id: String },
}

impl std::fmt::Debug for StrictInputRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("StrictInputRef(<opaque>)") }
}

impl StrictSourceOrigin {
    pub fn from_projection(session_id: String, projection: &xai_chat_state::InputProjection) -> Self {
        Self::from_receipt(session_id, projection.receipt_identity())
    }

    pub fn from_receipt(session_id: String, identity: xai_chat_state::input_projection::InputReceiptIdentity) -> Self {
        use xai_chat_state::input_projection::InputReceiptIdentity;
        match identity {
            InputReceiptIdentity::Completion { completions } => Self::Completion { session_id, completions },
            InputReceiptIdentity::CompletionToolResult { completions, tool_call_id } => Self::CompletionToolResult { session_id, completions, tool_call_id },
            InputReceiptIdentity::Notification { batch_id, completions } => Self::Notification { session_id, batch_id, completions },
            InputReceiptIdentity::Delivery { delivery_id } => Self::Delivery { session_id, delivery_id },
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum SourceBindingObservation {
    Input { notification: InputSourceNotification },
    Strict { identity: StrictSourceOrigin, binding: InputSourceBinding },
    Output { identity: OutputSourceOrigin, bindings: Vec<InputSourceBinding> },
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSourceOrigin {
    pub session_id: String,
    pub request_id: String,
    pub reference: xai_grok_sampling_types::OutputOrigin,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "granularity", rename_all = "snake_case")]
pub enum StreamOutputOrigin {
    Unknown,
    Known { identity: OutputSourceOrigin },
    Aggregate { origins: Vec<OutputSourceOrigin>, unidentified: usize },
}

impl std::fmt::Debug for StreamOutputOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("StreamOutputOrigin(<opaque>)") }
}

impl OutputSourceOrigin {
    pub fn into_meta(self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::from_iter([(OUTPUT_ORIGIN_META.to_owned(), serde_json::json!(StreamOutputOrigin::Known { identity: self }))])
    }
}

pub(crate) fn merge_output_meta(
    previous: Option<serde_json::Map<String, serde_json::Value>>,
    incoming: Option<serde_json::Map<String, serde_json::Value>>,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let previous_origin = previous.as_ref().and_then(|meta| meta.get(OUTPUT_ORIGIN_META));
    let incoming_origin = incoming.as_ref().and_then(|meta| meta.get(OUTPUT_ORIGIN_META));
    if previous_origin == incoming_origin { return previous; }
    let mut origins = Vec::new();
    let mut unidentified = 0usize;
    for value in [previous_origin, incoming_origin] {
        let origin = value.and_then(|value| serde_json::from_value::<StreamOutputOrigin>(value.clone()).ok()).unwrap_or(StreamOutputOrigin::Unknown);
        match origin {
            StreamOutputOrigin::Unknown => unidentified += 1,
            StreamOutputOrigin::Known { identity } => { if !origins.contains(&identity) { origins.push(identity); } }
            StreamOutputOrigin::Aggregate { origins: members, unidentified: missing } => {
                unidentified = unidentified.saturating_add(missing);
                for identity in members { if !origins.contains(&identity) { origins.push(identity); } }
            }
        }
    }
    let mut meta = previous.unwrap_or_default();
    meta.insert(OUTPUT_ORIGIN_META.to_owned(), serde_json::json!(StreamOutputOrigin::Aggregate { origins, unidentified }));
    Some(meta)
}

impl std::fmt::Debug for SourceBindingObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("SourceBindingObservation(<opaque>)") }
}

impl std::fmt::Debug for StrictSourceOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("StrictSourceOrigin(<opaque>)") }
}

impl std::fmt::Debug for OutputSourceOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("OutputSourceOrigin(<opaque>)") }
}
