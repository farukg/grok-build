use serde::{Deserialize, Serialize};

pub use xai_chat_state::{ContextItemRef, ToolCallId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveItemsRequest {
    pub items: Vec<ContextItemRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum RemoveContextItemsOutcome {
    Removed { items_removed: usize },
    TurnRunning,
    NotFound,
    BeforeCompaction,
    WouldOrphan,
}
