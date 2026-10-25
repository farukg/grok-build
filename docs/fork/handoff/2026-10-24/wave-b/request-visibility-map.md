# Map: request visibility (gb-request-visibility-10)

Codebase: `/home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover` (current truth, read-only inspection).
Requirement: everything that enters the model request must be visible in the CLI scrollback — collapsed by default, typed per surface, expanding to a Read-like body.

Related prior maps in this lane (zombie/tasks-pane visibility, gb-zombie-7):
`visibility-skeptic-map.md`, `lane-brief-visibility.md`. This map is the request-content
complement and does not overlap their scope.

## 0. Core finding (one paragraph)

The typing already exists: every runtime injection is persisted to chat-state as a
`ConversationItem::User/System` carrying a `SyntheticReason` variant
(`crates/codegen/xai-grok-sampling-types/src/conversation.rs:133-215`). Invisibility is created
almost entirely at two later points: (a) the pager **deliberately hides** those items from the
scrollback (`xai-grok-pager/src/acp/tracker.rs:1768-1791` text-prefix sniffing +
`PromptOrigin::hide_user_echo_from_scrollback`, `xai-grok-shell/src/session/mod.rs:264-279`), and
(b) `xai-chat-state` request assembly mutates the request view without persisting
(memory-reminder injection, pruning placeholders, context-policy projection:
`xai-chat-state/src/actor/request_builder.rs:34-87`). Fix = stop hiding (replace hide flags with a
kind tag the pager renders collapsed), finish tagging the four reserved-but-unwritten
`SyntheticReason` variants, and persist the one request-only injection.

## 1. Injection surfaces

### A. Persisted to chat-state (ConversationItem with SyntheticReason) — but hidden in scrollback

| # | Surface | Producer | Stored as | Hidden by |
|---|---------|----------|-----------|-----------|
| A1 | Harness `<system-reminder>` (todo-gate, date-rollover, workflow status, MCP-failed, skill/MCP catalog, fork/resume) | `session/acp_session_impl/reminders.rs:538,560` (`push_system_reminder[_with_tag]`, 8+ call sites in turn.rs/run_loop.rs/mcp.rs/session_mode.rs) | `UserItem` `SyntheticReason::SystemReminder` / `SkillCatalog` / `McpCatalog` | tracker.rs:1783 (`<system-reminder>` prefix) |
| A2 | Monitor events injected mid-turn | `acp_session_impl/turn.rs:1925-1944` (`inject_pending_monitor_events` → `ConversationItem::system_reminder`) | `UserItem` SystemReminder | tracker.rs:1784 (`<monitor-event` prefix) |
| A3 | Idle notification-drain wake (monitor + bash completions batched) | `acp_session_impl/notification_drain.rs:755-803` (`drain_notifications_into_turn`, `PromptOrigin::NotificationDrain`, `QueueMutationPolicy::hidden()`) | `UserItem` NotificationDrain | mod.rs:275 |
| A4 | Background bash-task completion wake | completion delivery path, `PromptOrigin::TaskCompleted` | `UserItem` TaskCompleted | mod.rs:272 (BgTaskBlock shows lifecycle, not payload) |
| A5 | Subagent completion wake | `PromptOrigin::SubagentCompleted` | `UserItem` SubagentCompleted | mod.rs:273 (SubagentBlock shows lifecycle, not payload) |
| A6 | Goal continuation directive (+ strategist note, reverify block) | `acp_session_impl/goal.rs:1498-1502` (`inject_goal_continuation_message` → `ConversationItem::goal_summary(directive)`), rendered by `goal_support.rs:377-575` | `UserItem` GoalSummary | mod.rs:276; pruning knows it (`goal.rs:1164-1183`) |
| A7 | Goal-achievement classifier nudge | `goal.rs:2102+` (`NotAchievedSyntheticReason`), `PromptOrigin::GoalClassifierNudge` | `UserItem` GoalClassifierNudge | mod.rs:277 |
| A8 | Stop-hook "keep working" feedback | `acp_session_impl/stop_gate.rs:236-367` (`run_stop_gate` → `announce_keep_working` status line only; body via `reminders.rs:544 wrap_hook_note`) | `UserItem` StopHookFeedback | reminder-tag rule; payload text invisible, one status line visible |
| A9 | Laziness/stall nudge | `acp_session_impl/laziness.rs` (3 `push_system_reminder` sites; `maybe_fire_laziness_check` :277) | `UserItem` SystemReminder | tracker.rs:1783 |
| A10 | Long-reasoning reminder | `turn.rs:2932-2950` (`take_due_reminder` → `push_system_reminder`) | `UserItem` SystemReminder | tracker.rs:1783 |
| A11 | Auto-continue / auto-recovery / length-continue | salvage + recovery paths | `UserItem` AutoContinue / AutoRecovery / LengthContinue | reminder framing; LengthContinue is join-keyed for report assembly (conversation.rs:146-150). Medium confidence on exact hide rule |
| A12 | Compaction summary sections | `helpers/session_compact.rs:433+` (`generate_session_compaction`) → `chat_state_handle.replace_conversation_for_compaction` (handle.rs:295); `CompactionMeta` re-read items (conversation.rs:140) | `SystemItem`/`UserItem` CompactionMeta | replay shows summary via ContextInfo-ish blocks; exact block mapping medium confidence |
| A13 | Memory section in system prompt | `acp_session_impl/prompt_build.rs:337-374` (`reconcile_resumed_memory_section`) mutates `SystemItem` head | mutated System head | never shown as scrollback content |
| A14 | Mid-turn interjection, parent/agent channel messages | `interjection.rs`, `parent_message.rs`, Delivery/AgentMessage/ParentHumanMessage origins | `UserItem` Interjection / AgentMessage / ParentHumanMessage | **visible** (control group — proof the pipeline can show synthetic items) |

### B. Goal-setup / session-prefix content (persisted, mistyped)

- `goal.rs:838-848` (`setup_goal`) and `:1030-1040` (resume) render rules via
  `goal_support.rs:250-342` but push them **untagged** as `Human` user items; `SyntheticReason::GoalSetup`
  is reserved ahead of its producer (conversation.rs:186-196, same for `SessionPrefix`, `DirectBash`).
  Visible in scrollback today but rendered as ordinary user prompts — needs the kind tag, not unhiding.

## 2. Request assembly (what flows ONLY through the request, never into chat-state)

Owner: `crates/codegen/xai-chat-state/src/actor/request_builder.rs::build_conversation_request`
(:19-118), invoked from `xai-grok-shell/src/session/acp_session_impl/turn.rs:2979-2983` via
`handle.rs:447-468` (`ChatStateCommand::BuildConversationRequest`, commands.rs:253-257).

Truly invisible today (request-only):
1. **Memory reminder, non-persist branch** — `request_builder.rs:34-45` persists only when
   `persist_memory_reminder` (turn.rs passes `self.memory.is_enabled()`); otherwise injected into
   the cloned items only at :75-76 via `inject_memory_reminder` (:204-227, splices text into the
   `System` head). The model sees it; chat_history.jsonl does not.
2. **Request-view transforms** — `prune_items_for_turn_request` (hard-clear placeholder :21,
   soft-trim separator :24 — old tool results are silently replaced in the request),
   `context_policy.project` + `withhold_memory_context` (:78-82),
   `compaction_utils::ModelRequestHistory::from_raw` folding (:87). These change request content
   relative to the stored history the CLI replays.
3. (Adjacent, not message content) tool-definition projection `sampler_turn.rs:346-410`.

## 3. Scrollback rendering

- Block model: `xai-grok-pager/src/scrollback/block.rs:25-51` `MessageKind` enum (UserPrompt,
  Read, Execute, …, BgTask, Subagent, Workflow, ContextInfo, MemoryCapture, Stub) — one variant
  per `RenderBlock`; `From<&RenderBlock>` maps automatically.
- Collapse: `scrollback/types.rs:65-91` `DisplayForm`/`DisplayMode` (Collapsed/Expanded/Truncated);
  per-kind default via `default_display_mode()` on each block; user form overrides stored per kind
  (`scrollback/state/kind_default.rs:13-20`, `clear_kind_default`).
- Reference UX: `ReadToolCallBlock` — `blocks/tool/read.rs:546-552` default `Collapsed`, finished
  also Collapsed; collapsed renders a one-line header, expanded renders styled file content via
  `render/content_kind.rs::styled_rows` (grammar/terminal-aware). Collapsed-entry rendering rules
  (one-line header, no rail, dimmed bullet, exact height estimate) live in
  `scrollback/wrappers/entry_renderer.rs:43,390,689-691,869-881,977,1516-1549`.
- Ingest path to extend: `acp/tracker.rs:1636-1706` builds `UserPromptBlock` from
  `SessionUpdate::UserMessageChunk` with meta flags (`user_message_chunk_meta::HIDE_FROM_SCROLLBACK`,
  INTERJECTION, DISPLAY_AS_CRON/SKILL, COMBINED_DISPLAY_TEXTS); the hide decision is
  `user_message_hidden_from_scrollback` (tracker.rs:1768-1791) + `PromptOrigin` rule
  (shell mod.rs:264-279). Hidden-but-recorded entries go to `turn_ledger.note_hidden_user_chunk`
  (tracker.rs:1727) for rewind bookkeeping only.
- Plain prompt fold precedent: `blocks/user.rs:16-19` (FOLD_THRESHOLD_LINES=3,
  COLLAPSED_RENDER_LINES=1).

## 4. Deliverable shape (minimal change, no patch)

Typing — reuse, do not invent: `SyntheticReason` (conversation.rs:133-215) already has one variant
per surface (≈22 live + 4 reserved). Work: make the four reserved producers real
(`GoalSetup` at goal.rs:838/1030, `SessionPrefix` at session_setup, `DirectBash`, `ParentHumanMessage`),
and carry the reason on the wire: a new `user_message_chunk_meta::SYNTHETIC_REASON` string on
UserMessageChunk (mirrors existing meta-flag pattern, tracker.rs:1646+).

Persistence-at-injection: everything in §1A/B is already persisted. The only gap is §2.1 —
`build_request`'s non-persist memory-reminder branch (request_builder.rs:75-76): persist the
injected system-head mutation (or emit a CompactionMeta-style marker item) so chat_history.jsonl
matches the request. Pruning/transform deltas (§2.2) are view derivations — show them as a
per-turn "context view" note rather than persisting.

Rendering — one new kind: `MessageKind::Injected` + `RenderBlock::Injected(InjectedBlock)` whose
payload is `(label_from_SyntheticReason, text)`; `default_display_mode() -> Collapsed`
(copied from read.rs:546); expanded body renders through the same `styled_rows` path Read uses;
collapsed header via existing entry_renderer collapsed rules. In tracker.rs, replace
`user_message_hidden_from_scrollback` + `PromptOrigin::hide_user_echo_from_scrollback` truth with
"map reason → InjectedBlock" (delete the text-prefix sniffing at :1783-1790); keep the rewind
ledger mapping via prompt_index.

Owners: shell (tag emission, memory-reminder persistence) · pager/tracker (kind mapping, block) ·
pager-render (entry rules already sufficient) · sampling-types (no change beyond reserved
variants). Invariants: KV-cache prefix stability (never replace placed items — same rule as
ProjectInstructions, conversation.rs:158-161), replay determinism (scrollback rebuilds from
chat_history.jsonl under kind defaults, kind_default.rs), `starts_prompt_turn` boundary semantics
(conversation.rs:226+) must keep classifying the newly-tagged items.

Gap/confidence notes: A11/A12 exact scrollback hide paths medium confidence; `queue_input`
(user prompt queue) and channel deliveries are visible today and out of scope.
