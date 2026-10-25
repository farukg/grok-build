# Wave B — lane brief 2+3 (combined author): request-visibility + message-display

One author owns both lanes — they share the scrollback/collapse machinery.
Maps (READ FIRST, binding):
- docs/fork/handoff/2026-10-24/wave-b/request-visibility-map.md
- docs/fork/handoff/2026-10-24/wave-b/message-display-map.md

## gb-request-visibility-10 (Faruk: "I want to see what the LLM sees")

14 invisible surfaces today: 12 persisted-but-hidden + 2 request-only
(memory-reminder splice, request-view transforms: pruning/context-policy folding).
The pager deliberately hides them (tracker.rs:1768-1791 text-sniffing +
PromptOrigin::hide_user_echo_from_scrollback, shell mod.rs:264-279) while
SyntheticReason typing already exists on persisted items.

Implement:
1. Injected entries render in the scrollback by default COLLAPSED, expandable
   Read-style, via kind defaults — NOT ad-hoc pinned modes. Zero new
   SyntheticReason kinds: finish the 4 reserved ones; add the one new
   `MessageKind::Injected` collapsed-by-default block (map proposal).
2. Remove the deliberate hiding for these origins: replace the text-sniffing
   suppression with the typed-kind routing so every persisted injected item shows
   (collapsed) instead of being dropped from scrollback.
3. The 2 request-only surfaces must become visible: persist a bounded injected
   record at splice/transform time (memory-reminder splice, request-view
   transforms) so the CLI can render it collapsed. Everything that enters the
   request must be CLI-visible — nothing enters model context invisibly.

## gb-msgdisplay-11

1. F7 "Message Display" defaults must hold across the WHOLE lifecycle: the two
   sites that override without consulting display_defaults —
   `finish_running_with_time` (state/mod.rs:1200-1203, e.g. Read forcing Collapsed
   on finish) and `replace_tool_block`'s kind-refinement reset (:1060-1066) — must
   resolve through one shared `ScrollbackState::resolve_kind_mode` helper lifted
   from `apply_kind_default` (:569/:637) and gated on display_defaults.
2. Assistant messages become foldable: content-driven `AgentMessageBlock::is_foldable`
   (agent.rs:192) + a Collapsed one-line branch in `output()` (agent.rs:148),
   following the Read/Btw pattern; the generic fold machinery (toggle_fold,
   selection.rs:279) stays as-is.

## Constraints

Author-only (no cargo/git/build/subprocesses); worktree
/home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover. You own the
pager scrollback/state/block files (tracker.rs, state/mod.rs, agent.rs,
selection.rs, entry kinds), chat-state typing for the injected record, and the
shell-side hiding/persist sites named in the map (mod.rs:264-279 +
request-assembly splice/transform sites). Other lanes own acp_session_impl
cancel/goal/stop_gate/run_loop, roster/dashboard, background_tasks/scheduler —
do not touch those. Tests: real entry points (kind resolution lifecycle,
foldable/expand, injected-record persistence + rendering decision logic) in
sibling *_tests.rs. Code rules: enums with payloads; no unwrap/expect/panic
outside tests; exhaustive matches; minimal diffs; comments only for non-obvious
constraints.

Write your report to docs/fork/handoff/2026-10-24/wave-b/lane-report-request-visibility.md.
Reply ≤10 lines: status, files changed, tests added, open questions. Never stop silently.
SUBAGENTS_ALLOWED: no; work silent.
