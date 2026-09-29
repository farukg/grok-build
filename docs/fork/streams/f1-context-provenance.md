# F1-core — context provenance for every category

Branch: create `grb/f1core` from `main` (the name is free on the fork).

## Goal
Every context category in `ContextCategory` (`ST/context_policy.rs`) can be switched off per
session from F7 and really changes the next model request (Faruk decision (b): everything is
switchable, only warnings). The current prompt and its attachments are always sent.
Design: `docs/fork/design/f6-f7.md` §1, §2, §7 and waves A1/A2/B1.

## Already on main
- `ContextCategory`, `RuntimeNotice`, `ContextSwitch`, `ContextPolicy`, `all_categories()`.
- `ConversationItem::context_category()` (`ST/context_provenance.rs`): exhaustive, derived from the
  variant and `SyntheticReason`; never stored.
- M5 per-item removal (`ContextItemRef`, replay persistence) for turns and tool exchanges.

## State (branch `grb/f1core`, from `main` a3bbf8a)
- A1 producers: `SessionPrefix` (prefix build in `session_setup.rs` / `rewrite_zero_turn_prefix`),
  `DirectBash` (`tool_dispatch.rs`), `GoalSetup` (`turn.rs`, `UserInputKind`). Readers now use
  `SyntheticReason::{is_user_input, is_legacy_human}` and `has_typed_session_prefix`, so turn counts and
  rewind cuts are unchanged for old (untyped) and new (typed) sessions.
  Deliberate small changes: transcripts/anchors for the turn summary, prompt suggestions and the
  laziness classifier no longer include the startup prefix text; `recent_user_asks`
  (`T/task/mod.rs`) skips `!cmd` messages now. Post-compaction the prefix is still `CompactionMeta`
  (`CS/compaction_utils.rs:932`), i.e. category `CompactionSummary`, not `Environment`.
- A2 projection: `ContextPolicy::project` (`ST/context_projection.rs`) removes items by category; current
  turn unchanged; tool calls/results/backend calls/definitions leave together (ToolDefinitions off drops
  exchanges too); reasoning stays only with a surviving output. Chat-state applies it in
  `build_conversation_request` (`CS/actor/request_builder.rs`) with `SetContextPolicy`; tool definitions
  are cleared when ToolDefinitions is off. Tests: `ST/context_projection_tests.rs`, `CS/actor/tests.rs`
  `context_policy_shapes_the_request_and_not_the_history`.

## Done since
- Child inheritance: a child starts under its resume source's saved policy, else its parent's live policy (snapshot, written to the child's
  session dir only when restricted, so the default path does no I/O; `context_policy_to_inherit` in `S/agent/subagent/mod.rs`). Hosted tools
  already follow ToolDefinitions (`turn.rs`).

## Next steps
1. B1 shell (remaining: `changed` notification): `x.ai/context/policy` get/set ext method, persist the policy in the session (replay, resume,
   compaction never resurrects excluded content because the filter is request-time; child inheritance);
   hosted tools and `tool_choice` in `turn.rs` (~L3003) must follow ToolDefinitions.
2. Mixed sections (`Sections::Split`) for the prefix (rules/skills/MCP/memory), system prompt fragments and
   the memory reminder; until then Skills/Mcp/Memory switches have no producer.
3. Pager F7 rows (see `streams/f2-f3-sidebars.md`).
