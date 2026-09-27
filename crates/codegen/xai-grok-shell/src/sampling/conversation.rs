//! Conversation types: re-exports the canonical set from `xai_grok_sampling_types` plus grok-shell-specific additions.

pub use xai_grok_sampling_types::conversation::*;

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;

/// Tracing context for conversation requests; satisfies `TraceContext` through its blanket impl.
/// Lives in grok-shell because it references shell-internal config and upload types.
#[derive(Debug, Clone)]
pub struct ConversationRequestTrace {
    pub gcs_config: crate::session::repo_changes::TraceExportConfig,
    #[expect(
        dead_code,
        reason = "retained for snapshot compat; wire when sampler path uploads traces"
    )]
    pub(crate) artifact_tracker: Option<crate::upload::manifest::ArtifactTracker>,
}

/// Filters chat history copied into a fork. Drops runtime-synthetic user messages, then cuts only the unfinished tail so the child never sees a partial exchange.
/// A turn is complete when the tool calls of its Assistant run (parallel calls may span consecutive Assistant items) are all answered; Reasoning and BackendToolCall items are transparent to the scan.
/// The compacted head (`CompactionMeta`, incl. the compaction summary) is the parent's only history right after a compaction, so it survives and closes like System.
/// An unfinished Assistant run is cut together with everything after it, but the user messages it was answering stay; trailing users nobody answered yet (e.g. the `/goal` prompt itself) are dropped.
/// Keep the "complete turn" definition in sync with `count_complete_turns` in `xai-grok-subagent-resolution/src/context.rs`.
pub(crate) fn fork_filter_chat(items: &mut Vec<ConversationItem>) {
    items.retain(|item| match item {
        ConversationItem::User(u) => {
            u.synthetic_reason.is_human() || u.synthetic_reason == SyntheticReason::CompactionMeta
        }
        _ => true,
    });

    // Only a complete Assistant run, System, or the compacted head advances the boundary; everything else is transparent.
    let refs: Vec<&ConversationItem> = items.iter().collect();
    let mut last_complete_end = 0;
    let mut i = 0;
    while let Some(item) = items.get(i) {
        match item {
            ConversationItem::System(_) => {
                last_complete_end = i + 1;
                i += 1;
            }
            ConversationItem::User(u) if u.synthetic_reason == SyntheticReason::CompactionMeta => {
                last_complete_end = i + 1;
                i += 1;
            }
            ConversationItem::Assistant(_) => {
                if let Some(end) =
                    xai_grok_subagent_resolution::context::complete_assistant_run_end(&refs, i)
                {
                    last_complete_end = end;
                    i = end;
                } else {
                    last_complete_end = i;
                    break;
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    items.truncate(last_complete_end);
}
