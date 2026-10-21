use serde::{Deserialize, Serialize};

pub use xai_chat_state::{ContextItemRef, ToolCallId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveItemsRequest {
    pub items: Vec<ContextItemRef>,
}

/// Result of `x.ai/session/remove_items`; the wire tag is `kind`.
///
/// `{"kind":"removed","removed":[{"kind":"tool_exchange","toolCallId":"call_a"}],"skipped":[{"item":{"kind":"turn","promptIndex":2},"reason":"not_in_context"}]}`
/// or a bare `{"kind":"turn_running"}` for the unit variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RemoveContextItemsOutcome {
    /// Everything the shell actually dropped from the model context, plus what it left alone and why.
    /// `removed` is never empty; a `Turn` ref covers the user item through the next prompt-indexed user item.
    Removed {
        removed: Vec<ContextItemRef>,
        skipped: Vec<SkippedItem>,
    },
    /// A turn is active; nothing changed.
    TurnRunning,
    /// No requested ref matched anything in the model context.
    NotFound,
    /// A requested turn lies at or below the compaction floor; the whole request was rejected.
    BeforeCompaction,
    /// Refs matched, but each was an unanswered or irregular tool exchange; nothing changed.
    WouldOrphan,
    /// The conversation changed in memory but could not be made durable; `removed` is what left the model context.
    PersistFailed { removed: Vec<ContextItemRef> },
}

/// A requested ref the shell left in the model context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedItem {
    pub item: ContextItemRef,
    pub reason: SkipReason,
}

/// Why a requested ref stayed in the model context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Neither the turn nor any call or result with that id is in the conversation.
    NotInContext,
    /// The call has no result yet.
    CallWithoutResult,
    /// The id occurs in a shape other than exactly one call and one result (or one backend call).
    AmbiguousId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_wire_shapes_are_stable() {
        let removed = RemoveContextItemsOutcome::Removed {
            removed: vec![ContextItemRef::ToolExchange {
                tool_call_id: "call_a".into(),
            }],
            skipped: vec![
                SkippedItem {
                    item: ContextItemRef::Turn { prompt_index: 2 },
                    reason: SkipReason::NotInContext,
                },
                SkippedItem {
                    item: ContextItemRef::ToolExchange {
                        tool_call_id: "call_b".into(),
                    },
                    reason: SkipReason::CallWithoutResult,
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(&removed).ok(),
            Some(serde_json::json!({
                "kind": "removed",
                "removed": [{"kind": "tool_exchange", "toolCallId": "call_a"}],
                "skipped": [
                    {"item": {"kind": "turn", "promptIndex": 2}, "reason": "not_in_context"},
                    {"item": {"kind": "tool_exchange", "toolCallId": "call_b"}, "reason": "call_without_result"},
                ],
            }))
        );
        assert_eq!(
            serde_json::to_value(RemoveContextItemsOutcome::PersistFailed {
                removed: vec![ContextItemRef::Turn { prompt_index: 1 }],
            })
            .ok(),
            Some(serde_json::json!({
                "kind": "persist_failed",
                "removed": [{"kind": "turn", "promptIndex": 1}],
            }))
        );
        assert_eq!(
            serde_json::to_value(RemoveContextItemsOutcome::TurnRunning).ok(),
            Some(serde_json::json!({"kind": "turn_running"}))
        );
        assert_eq!(
            serde_json::to_value(SkipReason::AmbiguousId).ok(),
            Some(serde_json::json!("ambiguous_id"))
        );
        let parsed = serde_json::from_value::<RemoveContextItemsOutcome>(serde_json::json!({
            "kind": "removed",
            "removed": [{"kind": "turn", "promptIndex": 0}],
            "skipped": [],
        }));
        assert_eq!(
            parsed.ok(),
            Some(RemoveContextItemsOutcome::Removed {
                removed: vec![ContextItemRef::Turn { prompt_index: 0 }],
                skipped: Vec::new(),
            })
        );
    }
}
