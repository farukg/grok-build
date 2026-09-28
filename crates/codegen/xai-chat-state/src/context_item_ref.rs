use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolCallId(Arc<str>);

impl ToolCallId {
    pub fn new(value: impl Into<Arc<str>>) -> Self { Self(value.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl From<&str> for ToolCallId {
    fn from(value: &str) -> Self { Self::new(Arc::<str>::from(value)) }
}

impl fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ContextItemRef {
    Turn { prompt_index: usize },
    ToolExchange { tool_call_id: ToolCallId },
}
