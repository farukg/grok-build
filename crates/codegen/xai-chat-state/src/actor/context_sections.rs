//! Context categories below item level: the memory manifest on the system head and split user items.

use std::collections::BTreeMap;
use std::sync::Arc;

use xai_grok_sampling_types::{CategoryTokens, ContentPart, ContextCategory, ConversationItem};

use super::request_builder::split_memory_context;
use super::state::estimate_item_tokens;

/// Takes the memory manifest out of the request's system head; a head that held only the manifest leaves.
pub(super) fn withhold_memory_context(items: &mut Vec<ConversationItem>) {
    let Some(ConversationItem::System(system)) = items.first_mut() else {
        return;
    };
    let (prompt, Some(_)) = split_memory_context(&system.content) else {
        return;
    };
    if prompt.is_empty() {
        items.remove(0);
    } else {
        system.content = Arc::from(prompt);
        system.evidence.record(xai_grok_sampling_types::ContentTransform::SystemHeadRewritten);
    }
}

/// Estimated tokens per category over `conversation`, counting each section under its own category.
pub(super) fn category_tokens(conversation: &[ConversationItem]) -> Vec<CategoryTokens> {
    let mut totals = BTreeMap::new();
    let mut add = |category: ContextCategory, tokens: u64| {
        *totals.entry(category).or_insert(0u64) += tokens;
    };
    for (index, item) in conversation.iter().enumerate() {
        match item {
            ConversationItem::System(system) if index == 0 => {
                let (prompt, manifest) = split_memory_context(&system.content);
                add(
                    item.context_category(),
                    xai_token_estimation::estimate_tokens(prompt),
                );
                if let Some(manifest) = manifest {
                    add(
                        ContextCategory::Memory,
                        xai_token_estimation::estimate_tokens(manifest),
                    );
                }
            }
            ConversationItem::User(user) if !user.sections.is_whole() => {
                for (part, category) in user.content.iter().zip(user.part_categories()) {
                    add(category, part_tokens(part));
                }
            }
            ConversationItem::System(_)
            | ConversationItem::User(_)
            | ConversationItem::Assistant(_)
            | ConversationItem::ToolResult(_)
            | ConversationItem::BackendToolCall(_)
            | ConversationItem::Reasoning(_) => {
                add(item.context_category(), estimate_item_tokens(item));
            }
        }
    }
    totals
        .into_iter()
        .map(|(category, tokens)| CategoryTokens { category, tokens })
        .collect()
}

fn part_tokens(part: &ContentPart) -> u64 {
    estimate_item_tokens(&ConversationItem::user_with_parts(vec![part.clone()]))
}
