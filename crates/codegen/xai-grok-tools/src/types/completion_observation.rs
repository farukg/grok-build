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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum ObservedTerminalCompletion {
    Task { snapshot: Box<crate::computer::types::TaskSnapshot> },
    Workflow { run_id: String, revision: u64 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ObservedCompletion {
    Subagent(ObservedSubagentCompletion),
    Terminal(ObservedTerminalCompletion),
}

impl ObservedCompletion {
    fn same_identity(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Subagent(a), Self::Subagent(b)) => a.subagent_id == b.subagent_id && a.attempt_id == b.attempt_id,
            (Self::Terminal(ObservedTerminalCompletion::Task { snapshot: a }), Self::Terminal(ObservedTerminalCompletion::Task { snapshot: b })) => a.task_id == b.task_id && a.owner_session_id == b.owner_session_id && a.kind == b.kind,
            (Self::Terminal(ObservedTerminalCompletion::Workflow { run_id: a, revision: ar }), Self::Terminal(ObservedTerminalCompletion::Workflow { run_id: b, revision: br })) => a == b && ar == br,
            _ => false,
        }
    }
}

mod attempt_serde {
    use super::*;
    pub fn serialize<S: Serializer>(attempt: &AttemptId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(attempt.as_str())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<AttemptId, D::Error> {
        let text = String::deserialize(deserializer)?;
        AttemptId::parse(&text).ok_or_else(|| serde::de::Error::custom("invalid observed subagent attempt"))
    }
}

/// Per-invocation output metadata, shared with nested meta-tool dispatches.
#[derive(Clone, Default)]
pub struct SubagentCompletionObservations(Arc<Mutex<Vec<ObservedCompletion>>>);

impl SubagentCompletionObservations {
    pub fn record(&self, result: &crate::implementations::grok_build::task::types::SubagentResult, attempt_id: &AttemptId) {
        let output: String = result.output.chars().take(4096).collect();
        self.extend(vec![ObservedCompletion::Subagent(ObservedSubagentCompletion {
            subagent_id: result.subagent_id.clone(), attempt_id: attempt_id.clone(),
            text: format!("Subagent {}/{}: {}\n{}", result.subagent_id, attempt_id, result.state.model_text(), output),
        })]);
    }
    pub fn record_task(&self, snapshot: &crate::computer::types::TaskSnapshot) {
        if snapshot.completed {
            self.extend(vec![ObservedCompletion::Terminal(ObservedTerminalCompletion::Task { snapshot: Box::new(snapshot.clone()) })]);
        }
    }
    pub fn record_workflow(&self, run_id: String, revision: u64) {
        self.extend(vec![ObservedCompletion::Terminal(ObservedTerminalCompletion::Workflow { run_id, revision })]);
    }
    pub fn extend(&self, observations: Vec<ObservedCompletion>) {
        let mut current = self.0.lock();
        for observation in observations {
            if !current.iter().any(|existing| existing.same_identity(&observation)) { current.push(observation); }
        }
    }
    pub(crate) fn stamp_error(&self, mut error: xai_tool_runtime::ToolError) -> xai_tool_runtime::ToolError {
        let mut details = match error.details.take() {
            Some(serde_json::Value::Object(details)) => details,
            Some(value) => serde_json::Map::from_iter([("toolDetails".into(), value)]),
            None => serde_json::Map::new(),
        };
        details.remove("_nativeSubagentCompletions");
        let observations = self.snapshot();
        if !observations.is_empty() && let Ok(value) = serde_json::to_value(observations) {
            details.insert("_nativeSubagentCompletions".into(), value);
        }
        error.details = (!details.is_empty()).then_some(serde_json::Value::Object(details));
        error
    }
    pub fn take_error(error: &mut xai_tool_runtime::ToolError) -> Vec<ObservedCompletion> {
        let value = error.details.as_mut().and_then(serde_json::Value::as_object_mut)
            .and_then(|details| details.remove("_nativeSubagentCompletions"));
        value.and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default()
    }
    pub fn snapshot(&self) -> Vec<ObservedCompletion> { self.0.lock().clone() }
}
