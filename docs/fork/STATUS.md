# Fork status

As of 2026-10-19. Upstream base: `origin/main` = `2bdd1d6a` (`xai-org/grok-build`).
`main` is the integration branch: everything that compiles and passes its tests lands here.
On 2026-10-19 Faruk ordered a one-time history fold: `main`'s fork delta over `2bdd1d6a`
was re-committed as the seven thematic fold commits below with a byte-identical tree.
The pre-fold history (all commit shas cited below as "pre-fold") is preserved in the local
`archive/pre-fold-main` branch and was the last state of the old remote `main`.

## Fold legend (main = `2bdd1d6a` + f1…f7)

| Group | Theme | Scope |
|---|---|---|
| f1 | build, CI and base files | `.github/` workflows, `rust-toolchain.toml`, `third_party`, `THIRD-PARTY-NOTICES`, root manifests |
| f2 | typed foundations | `xai-tool-types`, `xai-grok-sampling-types`, `xai-chat-state`, `xai-grok-subagent-resolution`, `xai-grok-shared`, `xai-grok-permission-rules`, `xai-grok-compaction` |
| f3 | shell: sessions, children, context | `xai-grok-shell`, `xai-grok-shell-base` |
| f4 | pager: views, timeline, trees, sidebars | `xai-grok-pager`, `xai-grok-pager-render`, `xai-grok-pager-pty-harness` |
| f5 | supporting crates | `xai-grok-markdown`, `xai-ratatui-textarea`, `xai-grok-sampler`, `xai-grok-config`, `xai-grok-agent`, `xai-grok-telemetry`, `xai-workflow` |
| f6 | grok_build tools | `xai-grok-tools` |
| f7 | fork docs and agent instructions | `CLAUDE.md`, `docs/fork/**` |

## On main

| Area | Fold group · evidence |
|---|---|
| M7 user prompts collapse to one line | f4 · `streams/w1h-hold-deliver.md` history |
| M8 subagent tokens in task rows | f4 |
| M10 goal planner: queue instead of abort, visible fork failure, forks keep summary | f3 |
| M9 real thinking levels and served model (requested → served) | f5, f6 |
| M1/M3 session switcher everywhere, deterministic subagent tree navigation, one row-to-child open path | f3, f4 |
| M4 subagent resume in every state | f3, f4 |
| M2 timeline outline over every entry, expandable F6 panel with scroll sync and item actions | f4 |
| Feedback tool gated by `features.feedback` | f6 |
| `/compact` "Send now" runs a queued agent command | f3 |
| Sibling cycling of subagents | f4 |
| Shared sidebar component + three-column geometry | f4 |
| Typed context policy (`ContextCategory`, `ContextPolicy`), whole-item classifier `ConversationItem::context_category()` | f2, f3 |
| Typed message display defaults (`MessageKind`, `DisplayForm`, `DisplayDefaults`, `EntryForm`) | f2, f4 |
| Shell: child session hosts in the registry | f3 |
| M11 typed subagent interruption state (types only) | f2, f6 · `streams/m11-subagent-control.md` |
| M5 remove turns / tool exchanges from the model context via the timeline, persisted across replay | f3, f4 |
| W2 subagent view = normal session view (child sessions are top-level views, takeover modal removed) | f3, f4 · `streams/w2-subagent-session-view.md` |
| W1-S human prompts reach child sessions by id, cancel by child id, `childSessions: "firstClass"` | f3 · `streams/w1s-child-sessions-shell.md` |
| W1-H a human prompt holds a running subagent's answer; `[deliver]` / F5 hands the last turn to the caller | f3, f4 · `streams/w1h-hold-deliver.md` |
| W1-S/W3 a finished or restart-lost child continues like a resume (`_meta.childResume`, pager stamps `_meta.childOf`) | f3, f4 |
| Fix: by-id handlers (set_model, compact, mode …) resolve a running child session | f3 |
| F1-core A1/A2/B1: typed origins, request projection under `ContextPolicy`, `x.ai/session/context_policy` get/set persisted, compaction summarizes what the policy sends | f2, f3 · `streams/f1-context-provenance.md` |
| W3 (part): pager knows `ShellChildSupport`, prompts from child views name their parent, a resumed finished child opens in place | f3, f4 · `streams/w3-pager-child-sessions.md` |
| F2/F3 (part): F7 sidebar with context switches, F6 timeline column, three-column layout, dock, Message Display rows | f4 · `streams/f2-f3-sidebars.md` |
| M11 B1: `SubagentResult.state` (typed `SubagentState`), interruption causes with actor, pause keeps a child wakeable, `x.ai/subagent/cancel` `mode: stop\|pause` | f2, f6 · `streams/m11-subagent-control.md` |
| Fix: resuming a finished subagent wakes it under its own id instead of starting a new one | f3 |
| M12 F6 outline and F7 sidebar as trees: arrows open/close/walk, Shift/Ctrl+Enter, Shift/Ctrl+click, `+ - *` bulk gestures | f4 · `design/f6-f7.md`, `views/tree_nav.rs` |
| M13 every F6/F7 action works or names the reason: removal verdict, `Removed { removed, skipped }`, `PersistFailed { removed }`, one removal in flight | f3, f4 · `app/agent_view/removal_verdict.rs`, shell `session/remove_items.rs` |
| M14 row colour and structure per kind, state and context presence | f4 · `views/row_style.rs` |
| M15 F6/F7 settings saved globally and per session (`panel-settings/`, `g` target, `t` turn default), restored after restart | f4 · `app/panel_settings/` |
| Fix: chat-state replaces the first answer when a tool call is answered twice (double `tool_result` → Anthropic 400) | f2 |
| CI: `linux-build` builds `grok` on every push to `main`; manual `linux-test` runs pager, shell-removal, chat-state and sampling-types tests | f1 |

Pre-fold test evidence (old history, `archive/pre-fold-main`): full `xai-grok-pager`,
`xai-grok-shell`, `xai-grok-sampling-types` runs green except the known failures below;
W2, W1-S, W1-H stream runs green; F6/F7 release build and new tests green in CI on
2026-10-19 (`51886e04` pre-fold); the chat-state fix green in build and manual test.
The fold is tree-identical: the same code, re-committed.

### Known failures (not regressions)
- Upstream, also failing on upstream `main`: `diagnostics::doctor_format::tests::limited_color_output_is_stable`,
  `doctor_cmd::tests::{human_mixed_fixture_is_exact, json_contract_is_structural_stable_ordered_and_ansi_free, json_empty_fixture_pins_null_policy}`
  (fixtures expect 6 themes, the tree has 7).
- Shell lib tests failing on `main` in the cloud container (environment, not code): four
  `agent::config::tests` api_backend/citation cases, `agent::subagent::tests::rest::read_parent_sampling_config_*` (2),
  `session::acp_session::mcp_file_input::tests::*` (3), `session::unified_list::tests::parse_list_req_forces_kind_under_process_chat_mode_only`.
  `agent::mvp_agent::tests::a_cancelled_installer_retires_its_thread_when_a_reinstall_takes_the_slot`
  overflows the default 2 MiB test stack and aborts the run: use `RUST_MIN_STACK=16777216`.
- Flaky: `app::acp_handler::tests::settings::settings_update_clearing_group_tool_verbs_reverts_to_default`.
  Appearance caches (`crate::appearance::cache::*`) are thread-local and seed from the developer's
  `[ui]` config on first read; tests that depend on them must set the value they need first.

## Continuing from the cloud (no access to the author's machine)

Cloud agents work purely from GitHub: `https://github.com/farukg/grok-build` (push remote `ours`).
Everything needed is on `main`: the fold put all landed work there; there are no stream branches to
fetch. Upstream `xai-org/grok-build` is `origin`; the fork base is `2bdd1d6a`.

- No local machine access: no cargo, no local grok runtime, no SIG tooling. GitHub Actions is the only build/test oracle: `linux-build` builds `grok` on every push to `main`; the manual `linux-test` workflow (workflow_dispatch) runs the pager, shell-removal, chat-state and sampling-types tests.
- Read this file first, then `docs/fork/skills/` (implementation bar), then the stream file for your task; designs in `design/`, review verdicts in `reviews/`, briefs in `briefs/`, recon in `recon/`.
- Branch workflow: cut `grb/<stream>` from `main` to work; once green, squash onto `main` and push (no PR); stream branches may be force-pushed with lease; `main` never.
- Expect the known failures listed above and the environment-dependent shell tests; `streams/f6-f7-trees.md` records what green means in CI.
- Commit-sha citations in the stream files and `plan/grb-001.md` refer to pre-fold history (`archive/pre-fold-main` locally); the fold legend above maps areas to the new fold commits.

## Work streams

| Stream | State | Stream file |
|---|---|---|
| W2 subagent view = normal session view | done — folded (f3, f4) | `streams/w2-subagent-session-view.md` |
| W1-H hold a steered subagent until `deliver` | done — folded (f3, f4); open low-prio: ↑ history in child views | `streams/w1h-hold-deliver.md` |
| W1-S shell: children are first-class ACP sessions | done — folded (f3); open questions in stream file | `streams/w1s-child-sessions-shell.md` |
| W3 pager consumes child sessions | on `main` (f3, f4); fork and roundtrip pinned by tests; open: set_model/compact/mode on a finished child (accepted) | `streams/w3-pager-child-sessions.md` |
| M11 agent + human control over subagents | B1 on `main` (f2, f6); next: `control_subagent` tool, `list_subagents`, pager pause/cause display | `streams/m11-subagent-control.md` |
| F1-display per-kind one-line renderers | Message Display rows on `main` (f2, f4); old renderer WIP rejected (`reviews/f1display.md`); open: better first lines per block, golden tests | `streams/f1-display.md` |
| F1-core context provenance | A1+A2+B1 on `main` (f2, f3); next: mixed sections (prefix/system fragments/memory) so Skills/MCP/Memory get rows | `streams/f1-context-provenance.md` |
| F2/F3 F6/F7 sidebars wired into the view | on `main` (f4); open: F6/F7 hang on very large sessions (see stream file), dock glyph/hover polish | `streams/f2-f3-sidebars.md` |
| F6/F7 trees, removal, colours, saved panel settings (GRB-001 M12–M15) | on `main` (f3, f4), CI green 2026-10-19; open points in the stream file | `streams/f6-f7-trees.md` |

Suggested order: the open W3 items, M11 and F1-display, then the F1-core mixed sections.
The planned context-control items (inspection, prompt editing, tool toggles, LLM view) are
GRB-001 S-N to S-U and not started. Before starting, cut your stream branch from `main`;
a stream is done only when `main` contains its progress: once green, squash it onto `main`
and push (no PR).

## Branch dispositions (2026-10-19 fold)

All work branches were old pre-rebase lineages (diverging at upstream `f0e3be11`); their landed
content reached `main` through squashes before the fold. Each was verified and deleted
(local and `ours`); `archive/pre-fold-main` keeps the old history locally.

| Branch (pre-fold tip) | Disposition · evidence |
|---|---|
| `grb/w2` (`686928b6`) | content on `main` via W2 squash (pre-fold `a632eab`); deleted |
| `grb/w2a` (`ab870205`) | ChildLink start-time increment already on `main` (cherry-pick applies empty); deleted |
| `grb/w2b` (`e26ec799`) | child-notification test increment already on `main` (cherry-pick applies empty); deleted |
| `grb/w2d` (`1c1111b2`) | interrupted WIP; its reconnect-test migration is superseded by `main`'s `find_by_session_id` API; deleted |
| `grb/w2e` (`8b96be85`) | interrupted WIP; unique delta is formatting-only (12 blank lines in `session_views.rs`); deleted |
| `grb/w1s` (`2bd214be`) | content on `main` via W1-S squash (pre-fold `c084eea`); deleted |
| `grb/w3` (`012fe214`, `ours`-only) | landed part on `main` via W3 squash; open W3 items are TODO, not in the branch; deleted |
| `grb/f2f3` (`9d331aa0`, `ours`-only) | landed part on `main` via F2/F3 squash; deleted |
| `grb/w1h` (`9e04aef5`, `ours`-only) | content on `main` (pre-fold `71a6fae`, `6fa71ae`); deleted |
| `fix/child-session-handlers` (`8ea97591`, `ours`-only) | by-id handler fix on `main` (pre-fold `8ea9759`); deleted |
| `claude/adoring-thompson-ap40n6` (`9d331aa0`, `ours`-only) | same tip as `grb/f2f3`; superseded; deleted |
| `grb/f1core` (`3b54b425`) | A1/A2/B1 on `main` (pre-fold `595f660d`, `3b54b425`); deleted |
| `grb/f1request` (`3b54b425`) | same tip as `grb/f1core`; request-projection worktree branch; deleted |
| `grb/f1display` (`8475aa9c`) | Message Display rows on `main`; old renderer WIP rejected (`reviews/f1display.md`); deleted |
| `grb/m11` (`bb27844e`) | B1 on `main` via M11 squash; open M11 items are TODO; deleted |

## Archive branches (history only, do not build on them)
- `archive/pre-fold-main` — the complete pre-fold `main` history (2026-10-19, tip `3da4bf09`+docs);
  every pre-fold sha cited in the docs resolves here. Local only.
- `archive/f1core-wip-sections` — earlier F1-core attempt with `Sections::{Whole, Split}` on items
  and a legacy wire test; reference for the mixed-section increment.
- `archive/f1core-wip-48671d59` — first F1-core attempt (rejected: stored a derived category).
- `archive/m11-full-wip` — M11 stash from before a rebase (superset of experiments).
- `archive/testfix-attempts-501aa7f0`, `archive/testfix-attempts-final` — rejected test-fix attempts.
- `archive/thinking-review-prerebase-20260926` — M9 state before its rebase.
