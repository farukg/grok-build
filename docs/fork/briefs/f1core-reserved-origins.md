# F1-core next increment brief (lead)

Start a branch from main. Previous work is preserved on `archive/f1core-wip-sections` (Sections/ContextSection model, legacy wire test) and `archive/f1core-wip-48671d59`.

## Already on main
- `ContextCategory`, `RuntimeNotice`, `ContextSwitch`, `ContextPolicy`, `all_categories()` in `xai-grok-sampling-types/src/context_policy.rs`.
- `ConversationItem::context_category()` in `xai-grok-sampling-types/src/context_provenance.rs`: exhaustive, derived from the variant plus `SyntheticReason`. `RuntimeNotice::CompactionMeta` was removed (compaction metadata is `ContextCategory::CompactionSummary`). Whole-item category is never stored.

## This increment: A1 producers for reserved origins (one commit per producer group)
The reserved `SyntheticReason::{SessionPrefix, DirectBash, GoalSetup, ParentHumanMessage}` exist but no producer emits them; today those items are `Human` and are misclassified as `UserTurns`.
1. **SessionPrefix**: the startup `<user_info>`/rules/VCS-status prefix item (`S/session/acp_session_impl/prompt_build.rs` ~525-675, `A/prompt/user_message.rs` ~207-241). Emit it with `SyntheticReason::SessionPrefix`. Audit every reader that finds the prefix "by position" or by `is_human()` (`starts_prompt_turn` already treats SessionPrefix as non-boundary) and switch them to the typed reason so behavior is identical — no turn-count, prompt_index, compaction or replay change.
2. **DirectBash**: the `!cmd` history message producer → `SyntheticReason::DirectBash` (it already starts a prompt turn, same as Human).
3. **GoalSetup**: the `/goal` rules+objective item → `SyntheticReason::GoalSetup`.
4. `ParentHumanMessage` only if a producer exists today; otherwise leave it reserved.
Each new constructor goes into `ConversationItem` beside the existing ones (same shape as `user_meta`, `system_reminder`).

Wire compatibility: these reasons already deserialize on old readers (`#[serde(other)] Unknown` fallback plus the reserved variants shipped). Persisted sessions written before this change keep `Human` for these items — acceptable; do not add load-time reclassification.

Mixed-content sections (`Sections::{Whole, Split}` from the wip branch) are the NEXT increment, only for items that truly carry several categories (system prompt fragments; the prefix if it bundles project rules/skills/MCP/memory). Do not start them in this increment; note in the result which producers need splitting, with file:line.

## Rules
- Faruk: make invalid states unrepresentable, one owner per concept, no catch-alls on owned enums, no unwrap/expect outside tests, no narrating comments, no unnecessary tests. Only behavior tests with an independent oracle (e.g. the prefix item carries SessionPrefix and is not counted as a user turn).
- Small commits; no AI attribution.
- Build: `cargo test -p xai-grok-sampling-types -p xai-chat-state` and `cargo check -p xai-grok-shell --tests`. Batch fixes per log.
- Record progress in `docs/fork/STATUS.md` (F1-core section).
