//! ConversationRequest assembly — image compaction, pruning, repair, memory injection.

use xai_grok_sampling_types::{
    ContextCategory, ConversationItem, ConversationRequest, ToolSpec, TraceContext,
};

use super::ChatStateActor;
use crate::events::ChatStateEvent;
use crate::image_budget::{
    ImageBudgetOutcome, apply_image_budget_with_limits, image_budget_limits,
};
use crate::types::{InjectedRequestKind, InjectedRequestRecord, PruningConfig};

/// Placeholder inserted when a tool result is hard-cleared.
/// `pub(super)` so `mutations.rs` can use the same string on the retained conversation.
pub(super) const HARD_CLEAR_PLACEHOLDER: &str = "[Tool result omitted — too old]";

/// Separator inserted between head and tail in soft-trimmed results.
const SOFT_TRIM_SEPARATOR: &str = "\n\n[…trimmed…]\n\n";

impl ChatStateActor {
    /// Build a `ConversationRequest` from current actor state (image eviction, prune, memory reminder).
    /// The command handler already ran integrity repair on the actor conversation before this clone.
    /// Do not re-run dangling/dedup repair on the clone — those would be O(n) no-ops.
    pub(super) fn build_conversation_request(
        &mut self,
        tool_definitions: Vec<ToolSpec>,
        memory_reminder: Option<String>,
        persist_memory_reminder: bool,
        trace: Option<Box<dyn TraceContext>>,
        conv_id: String,
        req_id: String,
    ) -> crate::types::BuiltConversationRequest {
        let mut injected_records = Vec::new();
        let mut memory_reminder = memory_reminder;
        let mut memory_record = None;
        if let Some(reminder) = memory_reminder.as_deref()
            && persist_memory_reminder
        {
            // A live in-place inject can prepend a `System` item, shifting indices
            // under an active capture; snapshot + rebase like the other mutators.
            self.snapshot_turn_slice();
            let injected = inject_memory_reminder(&mut self.state.conversation, reminder);
            if injected {
                self.persistence.replace_history(&self.state.conversation);
                memory_record = memory_reminder.take();
            }
            self.rebase_turn_capture_offset();
        }
        let (trigger_bytes, reclaim_target_bytes) =
            image_budget_limits(self.state.sampling_config.max_request_bytes);
        let budgeted = apply_image_budget_with_limits(
            self.state.conversation.clone(),
            trigger_bytes,
            reclaim_target_bytes,
        );
        let ImageBudgetOutcome {
            body_bytes,
            body_bytes_after,
            inline_images,
            needs_image_compaction,
            evicted,
        } = budgeted.outcome;
        let mut items = budgeted.items;
        if inline_images > 0 {
            self.send_event(ChatStateEvent::ImageBudget {
                body_bytes,
                trigger_bytes,
                reclaim_target_bytes,
                inline_images,
                needs_image_compaction,
                evicted,
                body_bytes_after,
            });
        }
        items = self.prune_items_for_turn_request(items);
        let pruned = tool_result_prune_delta(&self.state.conversation, &items);
        let mut replacement_bodies = String::new();
        if !pruned.is_empty() {
            for (stored, projected) in std::iter::zip(&self.state.conversation, &items) {
                if let (ConversationItem::ToolResult(stored), ConversationItem::ToolResult(projected)) =
                    (stored, projected)
                    && stored.content != projected.content
                {
                    replacement_bodies.push_str(&format!(
                        "\n\nTool result {}:\n{}", projected.tool_call_id, projected.content
                    ));
                }
            }
        }
        if let Some(reminder) = memory_reminder.as_deref()
            && inject_memory_reminder(&mut items, reminder)
        {
            memory_record = memory_reminder.take();
        }
        let memory_withheld = if !self.state.context_policy.includes(ContextCategory::Memory) {
            let had_manifest = matches!(items.first(), Some(ConversationItem::System(system))
                if split_memory_context(&system.content).1.is_some());
            super::context_sections::withhold_memory_context(&mut items);
            had_manifest
        } else {
            false
        };
        let projected_out = projected_out_categories(&self.state.context_policy);
        items = self.state.context_policy.project(items);
        let labeled_agent_messages = items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    ConversationItem::User(user)
                        if user.synthetic_reason
                            == xai_grok_sampling_types::SyntheticReason::AgentMessage
                )
            })
            .count();
        if let Some(mut summary) = context_view_summary(
            pruned,
            memory_withheld,
            &projected_out,
            labeled_agent_messages,
        ) {
            summary.push_str(&replacement_bodies);
            if labeled_agent_messages > 0 {
                summary.push_str("\n\nAgent-message prefix:\n");
                summary.push_str(crate::compaction_utils::AGENT_MESSAGE_MODEL_LABEL);
            }
            injected_records.push(InjectedRequestRecord::new(
                InjectedRequestKind::ContextView,
                &summary,
            ));
        }
        let tool_definitions = if self
            .state
            .context_policy
            .includes(ContextCategory::ToolDefinitions)
        {
            tool_definitions
        } else {
            Vec::new()
        };
        items = crate::compaction_utils::ModelRequestHistory::from_raw(items).into_items();
        if let Some(reminder) = memory_record.as_deref().map(str::trim)
            && matches!(items.first(), Some(ConversationItem::System(system))
                if system.content.contains(reminder))
        {
            injected_records.insert(0, InjectedRequestRecord::new(
                InjectedRequestKind::MemoryReminder,
                reminder,
            ));
        }
        for item in &items {
            if let Some(reason) = item.synthetic_reason()
                && !reason.is_human()
                && !reason.is_primary()
            {
                injected_records.push(InjectedRequestRecord::Synthetic {
                    reason: reason.clone(),
                    text: item.text_content(),
                });
            }
        }

        // Step 4: Assemble request
        let request = ConversationRequest {
            items,
            tools: tool_definitions,
            hosted_tools: vec![],
            tool_choice: None,
            model: Some(self.state.sampling_config.model.clone()),
            temperature: self.state.sampling_config.temperature,
            max_output_tokens: self.state.sampling_config.max_completion_tokens,
            top_p: self.state.sampling_config.top_p,
            x_grok_conv_id: Some(conv_id),
            x_grok_req_id: Some(req_id),
            x_grok_session_id: None,
            x_grok_turn_idx: None,
            x_grok_transient_retry: None,
            x_grok_agent_id: None,
            x_grok_deployment_id: None,
            x_grok_user_id: None,
            trace,
            traceparent: None,
            prompt_cache_key: None,
            reasoning_effort: self.state.sampling_config.reasoning_effort,
            json_schema: None,
            // Execute completed tool calls on a Length-truncated turn instead
            // of failing it; text-only salvage stays behind `CompletePartial`.
            length_policy: xai_grok_sampling_types::LengthPolicy::CompleteToolCalls,
        };
        crate::types::BuiltConversationRequest {
            request,
            injected_records,
        }
    }

    pub(super) fn prune_items_for_turn_request(
        &self,
        mut items: Vec<ConversationItem>,
    ) -> Vec<ConversationItem> {
        if should_prune(
            self.state.estimate_at_last_response + self.state.estimated_tokens_since_model,
            self.state.sampling_config.context_window,
        ) {
            prune_conversation(&mut items, &self.pruning_config);
        }
        items
    }
}

// ============================================================================
// Pruning (standalone functions, no actor state needed)
// ============================================================================

/// Check whether pruning should run based on context utilization.
///
/// Returns `true` when the retained conversation estimate exceeds 50% of `context_window`.
/// The provider-reported total is no input: it measures the pruned request, so gating on it flips pruning on and off every call.
pub(crate) fn should_prune(total_tokens: u64, context_window: std::num::NonZeroU64) -> bool {
    total_tokens > context_window.get() / 2
}

/// Prune old, large tool results from the conversation in place.
/// Turn age is estimated by walking backward and counting `User` items.
pub(crate) fn prune_conversation(conversation: &mut [ConversationItem], config: &PruningConfig) {
    if !config.enabled {
        return;
    }

    let mut turn_from_end: usize = 0;
    let mut seen_first_user = false;

    for item in conversation.iter_mut().rev() {
        if matches!(item, ConversationItem::User(_)) {
            if seen_first_user {
                turn_from_end += 1;
            }
            seen_first_user = true;
            continue;
        }

        let ConversationItem::ToolResult(tool_result) = item else {
            continue;
        };

        // Never prune recent turns.
        if turn_from_end < config.keep_last_n_turns {
            continue;
        }

        // Hard clear: very old tool results → replace entirely.
        if turn_from_end >= config.hard_clear_age_turns {
            if tool_result.content.as_ref() != HARD_CLEAR_PLACEHOLDER {
                tool_result.content = std::sync::Arc::<str>::from(HARD_CLEAR_PLACEHOLDER);
            }
            continue;
        }

        // Soft trim: large tool results → keep head + tail.
        let content_len = tool_result.content.chars().count();
        if content_len > config.soft_trim_threshold {
            let head = safe_char_slice(&tool_result.content, 0, config.soft_trim_head);
            let tail = safe_char_slice_tail(&tool_result.content, config.soft_trim_tail);
            tool_result.content =
                std::sync::Arc::<str>::from(format!("{head}{SOFT_TRIM_SEPARATOR}{tail}"));
        }
    }
}

// ============================================================================
// Request-view records (pure functions, no actor state needed)
// ============================================================================

/// How many tool results the request-view prune pass replaced, split by kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PruneDelta {
    /// Results replaced wholesale with [`HARD_CLEAR_PLACEHOLDER`].
    pub hard_cleared: usize,
    /// Results trimmed to head+tail around the soft-trim separator.
    pub soft_trimmed: usize,
}

impl PruneDelta {
    pub fn is_empty(&self) -> bool {
        self.hard_cleared == 0 && self.soft_trimmed == 0
    }
}

/// Compare the stored conversation against the pruned request view and count content replacements.
/// Only position-aligned pairs are compared: prune mutates in place, so any length mismatch caps
/// the count at the shorter side rather than misattributing deltas.
fn tool_result_prune_delta(
    stored: &[ConversationItem],
    pruned: &[ConversationItem],
) -> PruneDelta {
    let mut delta = PruneDelta::default();
    for (stored, pruned) in std::iter::zip(stored, pruned) {
        let (ConversationItem::ToolResult(stored), ConversationItem::ToolResult(pruned)) =
            (stored, pruned)
        else {
            continue;
        };
        if stored.content == pruned.content {
            continue;
        }
        if pruned.content.as_ref() == HARD_CLEAR_PLACEHOLDER {
            delta.hard_cleared += 1;
        } else {
            delta.soft_trimmed += 1;
        }
    }
    delta
}

/// Categories `ContextPolicy::project` actually rewrites, with the label the context-view record shows.
/// A category listed here as excluded means stored content in it did not reach the request verbatim.
fn projected_out_categories(policy: &xai_grok_sampling_types::ContextPolicy) -> Vec<&'static str> {
    use xai_grok_sampling_types::ContextCategory;
    let mut out = Vec::new();
    if !policy.includes(ContextCategory::Reasoning) {
        out.push("reasoning");
    }
    if !policy.includes(ContextCategory::AssistantTurns) {
        out.push("assistant turns");
    }
    if !policy.includes(ContextCategory::ToolExchanges) {
        out.push("tool exchanges");
    }
    out
}

/// One bounded line naming every request-view transform that changed content this turn.
/// `None` when the view matched the stored history — nothing entered the request invisibly.
fn context_view_summary(
    pruned: PruneDelta,
    memory_withheld: bool,
    projected_out: &[&str],
    labeled_agent_messages: usize,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if !pruned.is_empty() {
        parts.push(format!(
            "{} tool result(s) pruned ({} cleared, {} trimmed)",
            pruned.hard_cleared + pruned.soft_trimmed,
            pruned.hard_cleared,
            pruned.soft_trimmed
        ));
    }
    if memory_withheld {
        parts.push("memory context withheld".to_string());
    }
    if !projected_out.is_empty() {
        parts.push(format!("projected out: {}", projected_out.join(", ")));
    }
    if labeled_agent_messages > 0 {
        parts.push(format!(
            "{labeled_agent_messages} agent message(s) labeled as agent-authored"
        ));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

// ============================================================================
// Memory reminder injection
// ============================================================================

use crate::types::MEMORY_CONTEXT_OPEN_TAG;

/// Upsert a memory reminder into the conversation's system message.
/// Replaces a prior reminder section in-place, or prepends a `System` item if none exists.
/// Returns `true` when the conversation was changed.
pub(super) fn inject_memory_reminder(items: &mut Vec<ConversationItem>, reminder: &str) -> bool {
    let reminder = reminder.trim();
    if reminder.is_empty() {
        return false;
    }

    if let Some(ConversationItem::System(sys)) = items.first_mut() {
        upsert_memory_reminder_text(&mut sys.content, reminder)
    } else {
        items.insert(0, ConversationItem::system(reminder));
        true
    }
}

/// The system head as its prompt and, when one was injected, the memory manifest that follows it.
pub(super) fn split_memory_context(system: &str) -> (&str, Option<&str>) {
    match system
        .find(MEMORY_CONTEXT_OPEN_TAG)
        .and_then(|start| system.split_at_checked(start))
    {
        Some((prompt, manifest)) => (prompt.trim_end_matches('\n'), Some(manifest)),
        None => (system, None),
    }
}

fn upsert_memory_reminder_text(system_prompt: &mut std::sync::Arc<str>, reminder: &str) -> bool {
    let updated: String = if let (prefix, Some(_)) = split_memory_context(system_prompt) {
        if prefix.is_empty() {
            reminder.to_string()
        } else {
            format!("{prefix}\n\n{reminder}")
        }
    } else if system_prompt.trim_end() == reminder {
        system_prompt.as_ref().to_owned()
    } else if system_prompt.is_empty() {
        reminder.to_string()
    } else {
        format!("{}\n\n{reminder}", system_prompt.trim_end_matches('\n'))
    };

    if system_prompt.as_ref() == updated.as_str() {
        false
    } else {
        *system_prompt = std::sync::Arc::<str>::from(updated);
        true
    }
}

// ============================================================================
// String helpers
// ============================================================================

fn safe_char_slice(s: &str, start: usize, count: usize) -> String {
    s.chars().skip(start).take(count).collect()
}

fn safe_char_slice_tail(s: &str, count: usize) -> String {
    let total = s.chars().count();
    if count >= total {
        return s.to_string();
    }
    s.chars().skip(total - count).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_prune_gating() {
        use std::num::NonZeroU64;
        let cw = NonZeroU64::new(10000).unwrap();
        assert!(!should_prune(1000, cw)); // 10%
        assert!(should_prune(6000, cw)); // 60%
        assert!(!should_prune(5000, cw)); // 50% exact (> not >=)
    }

    #[test]
    fn prune_disabled_is_noop() {
        let mut conv = vec![ConversationItem::tool_result("c1", "x".repeat(10_000))];
        let config = PruningConfig {
            enabled: false,
            ..Default::default()
        };
        prune_conversation(&mut conv, &config);
        let [ConversationItem::ToolResult(tr)] = conv.as_slice() else {
            panic!("expected one tool result: {conv:?}")
        };
        assert_eq!(tr.content.len(), 10_000);
    }

    #[test]
    fn inject_memory_into_existing_system() {
        let mut items = vec![
            ConversationItem::system("You are helpful."),
            ConversationItem::user("hi"),
        ];
        inject_memory_reminder(&mut items, "Remember: user likes rust");
        if let Some(ConversationItem::System(sys)) = items.first() {
            assert!(sys.content.contains("Remember: user likes rust"));
            assert!(sys.content.starts_with("You are helpful."));
        }
        assert_eq!(items.len(), 2); // no new item added
    }

    #[test]
    fn inject_memory_prepends_when_no_system() {
        let mut items = vec![ConversationItem::user("hi")];
        inject_memory_reminder(&mut items, "Remember: user likes rust");
        assert_eq!(items.len(), 2);
        assert!(matches!(items.first(), Some(ConversationItem::System(_))));
    }

    #[test]
    fn prune_delta_classifies_hard_clears_and_soft_trims() {
        let stored = vec![
            ConversationItem::tool_result("c1", "unchanged"),
            ConversationItem::tool_result("c2", "very old original body"),
            ConversationItem::tool_result("c3", "large original body"),
            ConversationItem::user("turn"),
        ];
        let pruned = vec![
            ConversationItem::tool_result("c1", "unchanged"),
            ConversationItem::tool_result("c2", HARD_CLEAR_PLACEHOLDER),
            ConversationItem::tool_result("c3", "head\n\n[…trimmed…]\n\ntail"),
            ConversationItem::user("turn"),
        ];
        let delta = tool_result_prune_delta(&stored, &pruned);
        assert_eq!(delta.hard_cleared, 1);
        assert_eq!(delta.soft_trimmed, 1);
        assert_eq!(tool_result_prune_delta(&stored, &stored), PruneDelta::default());
    }

    #[test]
    fn context_view_summary_omits_empty_transforms() {
        assert_eq!(
            context_view_summary(PruneDelta::default(), false, &[], 0),
            None,
            "a view identical to the stored history produces no record"
        );
        let summary = context_view_summary(
            PruneDelta {
                hard_cleared: 2,
                soft_trimmed: 1,
            },
            true,
            &["reasoning"],
            3,
        )
        .unwrap();
        assert!(summary.contains("3 tool result(s) pruned (2 cleared, 1 trimmed)"));
        assert!(summary.contains("memory context withheld"));
        assert!(summary.contains("projected out: reasoning"));
        assert!(summary.contains("3 agent message(s) labeled"));
    }

}
