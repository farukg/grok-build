use crate::{ContextCategory, ContextPolicy, ConversationItem};

impl ContextPolicy {
    /// The history one request sends under this policy.
    ///
    /// The current turn (from the newest prompt-starting user item on) is sent unchanged, so a switch
    /// never strands a turn that is already running. Tool calls, their results and backend calls leave
    /// together, and so do the tool definitions they need. A reasoning item stays only while an output
    /// item of its response stays.
    pub fn project(&self, items: Vec<ConversationItem>) -> Vec<ConversationItem> {
        if self.is_unrestricted() {
            return items;
        }
        let current_turn = items
            .iter()
            .rposition(|item| {
                matches!(item, ConversationItem::User(user) if user.synthetic_reason.starts_prompt_turn())
            })
            .unwrap_or(items.len());
        let exchanges = self.includes(ContextCategory::ToolExchanges)
            && self.includes(ContextCategory::ToolDefinitions);
        let mut projected = Vec::with_capacity(items.len());
        let mut response = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            if index >= current_turn {
                projected.push(item);
                continue;
            }
            match item {
                ConversationItem::Reasoning(_) => response.push(item),
                ConversationItem::BackendToolCall(_) => {
                    if exchanges {
                        response.push(item);
                    }
                }
                ConversationItem::Assistant(mut assistant) => {
                    if !self.includes(ContextCategory::AssistantTurns) {
                        assistant.content = "".into();
                    }
                    if !exchanges {
                        assistant.tool_calls.clear();
                    }
                    let speaks = !assistant.content.is_empty() || !assistant.tool_calls.is_empty();
                    let answers = speaks
                        || response
                            .iter()
                            .any(|kept| matches!(kept, ConversationItem::BackendToolCall(_)));
                    for kept in response.drain(..) {
                        let reasoning = matches!(kept, ConversationItem::Reasoning(_));
                        if !reasoning || (answers && self.includes(ContextCategory::Reasoning)) {
                            projected.push(kept);
                        }
                    }
                    if speaks {
                        projected.push(ConversationItem::Assistant(assistant));
                    }
                }
                ConversationItem::ToolResult(_) => {
                    if exchanges {
                        projected.push(item);
                    }
                }
                ConversationItem::System(_) | ConversationItem::User(_) => {
                    if self.includes(item.context_category()) {
                        projected.push(item);
                    }
                }
            }
        }
        projected
    }
}

#[cfg(test)]
#[path = "context_projection_tests.rs"]
mod tests;
