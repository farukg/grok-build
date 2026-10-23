use parking_lot::Mutex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;
use xai_message_delivery_core::AttemptId;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObservedSubagentCompletion {
    pub subagent_id: String,
    pub text: String,
    #[serde(with = "attempt_serde")]
    pub attempt_id: AttemptId,
}

mod attempt_serde {
    use super::*;
    pub fn serialize<S: Serializer>(attempt: &AttemptId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(attempt.as_str())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<AttemptId, D::Error> {
        let text = String::deserialize(deserializer)?;
        AttemptId::parse(&text)
            .ok_or_else(|| serde::de::Error::custom("invalid observed subagent attempt"))
    }
}

/// Per-invocation output metadata, shared with nested meta-tool dispatches.
#[derive(Clone, Default)]
pub struct SubagentCompletionObservations(Arc<Mutex<Vec<ObservedSubagentCompletion>>>);

impl SubagentCompletionObservations {
    pub fn record(
        &self,
        result: &crate::implementations::grok_build::task::types::SubagentResult,
        attempt_id: &AttemptId,
    ) {
        let mut observations = self.0.lock();
        if !observations
            .iter()
            .any(|o| o.subagent_id == result.subagent_id && &o.attempt_id == attempt_id)
        {
            let output: String = result.output.chars().take(4096).collect();
            let text = format!(
                "Subagent {}/{}: {}\n{}",
                result.subagent_id,
                attempt_id,
                result.state.model_text(),
                output
            );
            observations.push(ObservedSubagentCompletion {
                subagent_id: result.subagent_id.clone(),
                attempt_id: attempt_id.clone(),
                text,
            });
        }
    }
    pub fn extend(&self, observations: Vec<ObservedSubagentCompletion>) {
        let mut current = self.0.lock();
        for observation in observations {
            if !current.iter().any(|existing| {
                existing.subagent_id == observation.subagent_id
                    && existing.attempt_id == observation.attempt_id
            }) {
                current.push(observation);
            }
        }
    }
    pub(crate) fn stamp_error(
        &self,
        mut error: xai_tool_runtime::ToolError,
    ) -> xai_tool_runtime::ToolError {
        let mut details = match error.details.take() {
            Some(serde_json::Value::Object(details)) => details,
            Some(value) => serde_json::Map::from_iter([("toolDetails".into(), value)]),
            None => serde_json::Map::new(),
        };
        details.remove("_nativeSubagentCompletions");
        let observations = self.snapshot();
        if !observations.is_empty()
            && let Ok(value) = serde_json::to_value(observations)
        {
            details.insert("_nativeSubagentCompletions".into(), value);
        }
        error.details = (!details.is_empty()).then_some(serde_json::Value::Object(details));
        error
    }
    pub fn take_error(error: &mut xai_tool_runtime::ToolError) -> Vec<ObservedSubagentCompletion> {
        let value = error
            .details
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|details| details.remove("_nativeSubagentCompletions"));
        value
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }
    pub fn snapshot(&self) -> Vec<ObservedSubagentCompletion> {
        self.0.lock().clone()
    }
}
