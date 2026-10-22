# F1-core — context provenance for every category

Squashed onto `main` (the branch `grb/f1core` is history).

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

## State
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
- B1 shell: `x.ai/session/context_policy` get/set, persisted per session (`context_policy.json`, written only on edit), compaction
  summarizes what the policy sends. Open: a `changed` notification so other clients follow.
- Pager F7 rows: per-category ON/OFF with estimated tokens, acknowledged by the shell (`streams/f2-f3-sidebars.md`).
- Child inheritance: a child starts under its resume source's saved policy, else its parent's live policy (snapshot, written to the child's
  session dir only when restricted, so the default path does no I/O; `context_policy_to_inherit` in `S/agent/subagent/mod.rs`). Hosted tools
  already follow ToolDefinitions (`turn.rs`).

- Mixed sections (branch `grb/f1core`, squashed onto `main`): every category has a producer, so F7 lists all
  of them (`switchable_categories()` is gone; rows = `all_categories()`).
  - `UserItem.sections: Sections::{Whole, Split(Vec<ContextCategory>)}` (`ST/context_provenance.rs`), one category per
    content part; `Whole` is the serde default and is never written, so old sessions load and re-serialize byte-identical.
    The whole-item category stays derived (`context_category()`); only the split is stored.
  - Startup prefix (Default template): `build_user_message_prefix` returns the typed item
    `ConversationItem::session_prefix_with_rules(environment, rules)` = `[Environment, ProjectInstructions]`
    (`S/session/acp_session_impl/prompt_build.rs`). Rewrite/insert paths take the item as is.
  - Skills/workflow catalog and MCP server listing: whole items with new origins `SyntheticReason::{SkillCatalog,
    McpCatalog}` (baseline + update skill reminder in `session_setup.rs`, MCP reminder in `mcp.rs`).
    `SyntheticReason::is_system_reminder()` keeps the two startup strips (`session_setup.rs`, `prompt_build.rs`)
    removing the same items as before.
  - Memory: the injected `<memory-context>` manifest on the system head is derived, not stored: chat-state is its only
    producer and the tag is its canonical delimiter (`CS/types.rs` `MEMORY_CONTEXT_OPEN_TAG`, split once by
    `split_memory_context` in `CS/actor/request_builder.rs`, shared with the upsert). Memory OFF withholds it
    from the request (`CS/actor/context_sections.rs` `withhold_memory_context`); the canonical history keeps it. Storing a split on `SystemItem` was rejected: seven head writers replace
    `sys.content` directly (`prompt_build.rs:324,356`, `session_mode.rs:156`, `model_switch.rs:135,139`,
    `CS/conversation_util.rs:29`, `ST/conversation.rs` `transform_conversation_cwd`) and would leave a stale split.
  - Usage (`x.ai/session/context_policy` report): `category_tokens` counts each section under its own category.
  - Tests: `ST/context_projection_tests.rs` (catalogs per category, split prefix, reload + old wire),
    `CS/actor/tests.rs` `memory_off_withholds_the_manifest_and_keeps_the_history`,
    `context_usage_counts_each_section_under_its_own_category`.

## Verified defects and TODO

- [ ] Verify the implemented F7 function-tool accounting fix in GitHub CI and the staged artifact. `CS/actor/queries.rs::context_usage` counts only conversation items; schemas are supplied separately. The shell report now adds their estimate using the same child-safe tool projection as inference. A regression test covers schemas outside history and the disabled row's retained potential cost. Hosted-tool accounting and fresh/resumed/child UI verification remain open.
- [ ] Repair provenance for existing session prefixes and skill catalogs. Session `01a156e0-6d32-7921-a14a-f77e1b67e1be` contains workspace rules and a skill catalog while F7 reports Project rules/Skills as zero and attributes their content to Environment/SystemReminder. Preserve authored content and validate request filtering as well as counts; keep ambiguous legacy content explicit.
- [ ] Verify provenance across compaction and custom templates using the same category owners as request projection.
- [ ] Diagnose intermittent tool-availability denials on `sigma/openai/gpt-6.1-sol`. Failing session `01a1630d-01e3-72d3-a159-c84308b6dfd8` forwarded all 29 declarations through the gateway. Two fresh CLI probes with identical outbound bodies except `prompt_cache_key` produced a denial and a successful `pwd` call. Repeated cache-key probes produced both outcomes with and without a key. F7 zero counts are a separate reporting defect; upstream visibility and request-shape causality remain unresolved.

Session `01a16318-1171-7422-88a6-bdf49e87b11a` eventually called tools after context toggles/compaction. Its first `read_file` used `path`; runtime validation required `target_file`, and the corrected call succeeded. `list_dir` similarly corrected `directory` to `target_directory`. This proves execution and argument validation work; it does not prove which context operation changed the model's behavior.

## Next steps
1. B1 shell, remaining: the `changed` notification.
2. Custom `user_message_template` prefixes (MiniJinja) render rules/skills/MCP in one string and stay `Whole`
   (`Environment`); splitting them needs section output from `UserMessageContext::render` (`A/prompt/user_message.rs`).
3. Post-compaction the prefix is re-rendered as `CompactionMeta` text (`CS/compaction_utils.rs:932`), so the split
   ends at the first compaction (as before for `Environment`).

## Open questions
- Old binaries read the new origins `skill_catalog`/`mcp_catalog` as `Unknown`, which counts as a prompt-turn
  boundary there (`ST/conversation.rs` `starts_prompt_turn`); new binaries read every old session unchanged.
- Workflow launch/status/completion reminders stay `SystemReminder` (runtime notices, not catalog), per design §1.
- Cross-compaction rewind restores the prefix as a `Human` item (`S/session/acp_session_impl/rewind.rs:333`,
  pre-existing); the captured text now keeps every prefix part (`S/session/compaction.rs`).
