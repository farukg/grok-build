# Fork status

Current delivery status: 2026-10-26. The fork is `farukg/grok-build` (`origin`);
`xai-org/grok-build` is upstream, not the push destination.

Source/artifact commit `479d1652cf8b73cb2decbaf8fbc12a3bacd44f7f` passed GitHub
Actions [linux-build run 40247692299](https://github.com/farukg/grok-build/actions/runs/40247692299).
The exact-run artifact was staged: checksum validation succeeded, the release binary
was placed at `/home/fg/.cache/cargo-target/quick/release/xai-grok-pager`, and the
Grok deployment stage reported 1 promoted, 0 skipped, with installed target
`/home/fg/.cargo/bin/grok`. This records build and stage evidence only; no process
restart, running-binary identity, runtime/PTY acceptance, or tests were performed.
The earlier build attempt [run 40242190071](https://github.com/farukg/grok-build/actions/runs/40242190071)
failed on intermediate commit `5f50af26`; it was superseded by the successful run
above. Preserved refs `875ff73c` and `e7ceeb50` remain available.

The subsequent docs-only commit is not the source/artifact commit and has not itself
been built. Keep artifact provenance tied to the exact successful Run/Commit record.

Historical published `main` at `ba47d59ffc2538a6f4138ce61fd186376776fe21` was an
independently source-reviewed Root/Child intermediate. GitHub Actions
[linux-build run 40178088795](https://github.com/farukg/grok-build/actions/runs/40178088795)
completed successfully at that exact SHA (created 2026-10-25T08:48:29Z), automatically
triggered by the intermediate push. This is historical build-only evidence;
same-SHA `linux-test` acceptance was unverified and it does not establish current
main's build or acceptance.

**Effective policy: checks, tests, formatters, validation scripts and hooks remain
disabled locally and in GitHub Actions. Required release-artifact builds run solely
in GitHub Actions. Local builds, `sig build` and sig-orch communication remain disabled.** No force
push is authorized on any branch. Historical local results and older handoff
instructions do not override this policy.

The [owner-takeover plan](plan/owner-takeover.md) owns active delivery and acceptance
details; its existing [handoff](handoff/2026-10-24/owner-handoff/handoff.md) and
[Wave B references](handoff/2026-10-24/wave-b/) retain requirements and history.
Current evidence supersedes their older local-verification and gateway-owner text.

## Local follow-up source work (2026-10-26)

F6 selection safety now binds removal confirmations to the original agent, session,
stable row and exact context refs. F6 rewind requires an explicitly selected numbered
Turn, always confirms its whole-tail scope with No selected initially, and rejects
stale targets. Missing rewind points and disappearing outline rows have no replacement
default. Contract: [F6 actions and context removal](design/f6-f7.md).
Regression cases are authored and unexecuted; these local changes have no build or
runtime/PTY acceptance evidence. Current instruction: local thematic commits only.

## Current source disposition and open gates

- R: local terminal execution identity, shutdown enrollment/kill settlement and
  collection-error escalation repairs are source-closed. ACP detached-owner execution,
  registered-resource teardown and consumption semantics are also source-closed.
  Runtime behavior and tests remain unverified.
- D: durable receiver teardown, cancellation ownership, lease lifetime, ACK settlement
  and recovery/fault-scope repair are source-closed in the begun work. Runtime behavior
  and tests remain unverified.
- S: the goal-active suppression route remains intact; mixed notification-batch
  ownership, unrelated transient MonitorEvent preservation and batch retry behavior are
  source-closed. Runtime behavior and tests remain unverified. Broader S/R/D integration
  windows and restart-policy provenance limitations remain open as documented by the
  source reports.
- Compaction: bounded invocation-correlation source work is present; replay/persistence
  oracles remain unexecuted. Legacy untagged producer ambiguity and capability/schema
  compatibility remain external gates. Maomao is the current sigma-crates owner;
  schema/pass-through, correlated producer availability and consumer-first rollout
  acceptance are unverified. SIG delivery-outage diagnosis is separate; no shared
  restart is authorized.
- Source closure is not test discovery, runtime, or UI acceptance. Source/artifact
  commit `479d1652cf8b73cb2decbaf8fbc12a3bacd44f7f` has successful release-build
  evidence in run `40247692299` and exact-run staging evidence above. No tests,
  process restarts, or runtime/PTY verification were performed; the currently running
  binary is unverified. No additional checks or tests are authorized in this scope.

## Historical fold baseline

The 2026-10-19 fold used upstream base `2bdd1d6a`. The following inventory preserves
previously delivered capabilities and historical evidence, not current-wave verification.
Pre-fold references use `archive/pre-fold-main` as the recorded historical retrieval owner;
its present local availability has not been checked.

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
| W1-H automatic completion by default; explicit child header / F5 manual toggle and caller dock warning (current author-only update awaits CI) | f3, f4 · `streams/w1h-hold-deliver.md` |
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
| LSP attach (config-driven backend, tool only with a server) + preserving file-acceleration factory install with session gate; pager-global F6/F7 chrome with focus-coloured frames (minimal sessions defer adoption), prompt PgUp/PgDn paging incl. overlays, Home/End = previous user message / bottom+follow, Ctrl+X unbound from stop/close and opens the cheatsheet, dock keyboard-focus row, `/dock` runtime toggle persisting `features.dock` | f2, f3, f4, f6 · goal session 2026-10-20, skeptic-verified from source |

Pre-fold test evidence (old history, `archive/pre-fold-main`): full `xai-grok-pager`,
`xai-grok-shell`, `xai-grok-sampling-types` runs green except the known failures below;
W2, W1-S, W1-H stream runs green; F6/F7 release build and new tests green in CI on
2026-10-19 (`51886e04` pre-fold); the chat-state fix green in build and manual test.
The fold is tree-identical: the same code, re-committed.

### Historical known failures (not current-source verdicts)

Later handoff evidence at `bc85ceb9` records a green build and 27 distinct failed
test identities plus two hang entries: 28 unique identities, because the PTY case
appears as both failure and hang. At Wave A baseline `5abbd3c`, the build succeeded
but the test run was cancelled after authorized abort of the stalled step. Cancellation
is neither test failure nor acceptance. The unavailable job log and historical exit
code 143 do not identify a product cause or the stalled command.

The telemetry failure `execute_tool_calls_records_the_product_row_and_execution_span`
has no individual assertion stdout; its exact historical cause remains unknown.
A historical isolated pass does not establish cause or current acceptance. Typed
wake/ACK and serialized Summary fixture repairs are present at source level, but
shared helper repairs do not prove every historical failure fixed. Reconciliation
of the independent `SubagentCompleted` shutdown assertion and producer terminal
persistence for `finished_coordinator_child_compacts_persisted_checkpoint_without_parent_delivery`
(historical `NotTerminal`) remains open. PTY navigation hints were updated at source;
runtime acceptance is unverified. See the
[plan's historical evidence](plan/owner-takeover.md#input-and-historical-evidence).
- Upstream, also failing on upstream `main`: `diagnostics::doctor_format::tests::limited_color_output_is_stable`,
  `doctor_cmd::tests::{human_mixed_fixture_is_exact, json_contract_is_structural_stable_ordered_and_ansi_free, json_empty_fixture_pins_null_policy}`
  (fixtures expect 6 themes, the tree has 7).
- Earlier status attributed these cloud-container shell failures to environment:
  four `agent::config::tests` api_backend/citation cases,
  `agent::subagent::tests::rest::read_parent_sampling_config_*` (2),
  `session::acp_session::mcp_file_input::tests::*` (3), and
  `session::unified_list::tests::parse_list_req_forces_kind_under_process_chat_mode_only`.
  These remain historical environment candidates, not proven non-code failures.
  Current evidence does not establish environment prerequisites for the seven later
  config/MCP/unified-list cases; ambient configuration alone does not prove cause.
  Neither a blanket seven-environment classification nor complete current closure
  is supported.
  `agent::mvp_agent::tests::a_cancelled_installer_retires_its_thread_when_a_reinstall_takes_the_slot`
  historically overflowed the default 2 MiB test stack and aborted the run;
  the recorded CI stack requirement is `RUST_MIN_STACK=16777216`, not local execution permission.
- Flaky: `app::acp_handler::tests::settings::settings_update_clearing_group_tool_verbs_reverts_to_default`.
  Appearance caches (`crate::appearance::cache::*`) are thread-local and seed from the developer's
  `[ui]` config on first read; tests that depend on them must set the value they need first.

## Continuing delivery

Use the [owner-takeover plan](plan/owner-takeover.md), preserving existing author
source and canonical documentation for integration. Not all required work is on
published `main`. Push normally only to `farukg/grok-build` (`origin`); no force
push is authorized, including force-with-lease on stream branches.

The release workflow is artifact-build/package-only; formatting guard execution stays
in `linux-test`, not `linux-build`. Current integration remains in progress and has
no final SHA. The current delivery instruction authorizes only the necessary GitHub
release build for current `main`; no tests, local checks, formatters or `sig build`
are authorized. Stage only the exact current-main artifact after that release build
succeeds. Historical failure evidence above must not be treated as current acceptance.

Historical stream references and `plan/grb-001.md` remain useful capability/design
owners, not authority to restart obsolete branch or local-verification workflows.

## Context reporting and tool-call diagnosis

Historical F7 function-tool schema accounting and native Responses Lite findings
remain documented in `streams/f1-context-provenance.md` under “Verified defects and TODO”.
Hosted-tool accounting and fresh/resumed/child UI acceptance remain unverified.
Existing session prefixes/catalogs can put workspace rules under Environment and
skills under SystemReminder, leaving their intended rows at zero. Historical native
stream reduction needed to assemble `output_item.done` items when terminal `output`
is empty to avoid repeated successful text turns. None of these historical artifact
references establishes the currently selected or running binary.

Intermittent model claims of missing tools are reproduced independently of those display errors: the gateway's outbound capture contains all 29 declarations, and identical fresh-session payloads can produce either a denial or a successful call. A causal regression in Grok CLI or the gateway has not yet been isolated.

## Work streams

| Stream | State | Stream file |
|---|---|---|
| W2 subagent view = normal session view | done — folded (f3, f4) | `streams/w2-subagent-session-view.md` |
| W1-H explicit automatic/manual completion delivery | author-only update; authoritative completion/caller dock warning integrated; Lead CI pending | `streams/w1h-hold-deliver.md` |
| W1-S shell: children are first-class ACP sessions | done — folded (f3); open questions in stream file | `streams/w1s-child-sessions-shell.md` |
| W3 pager consumes child sessions | on `main` (f3, f4); fork and roundtrip pinned by tests; open: set_model/compact/mode on a finished child (accepted) | `streams/w3-pager-child-sessions.md` |
| M11 agent + human control over subagents | B1+B2+B3 and list/outcome visibility on `main` (f2, f6); open: `SubagentFinished` carries `interruption`, pager pause/cause display | `streams/m11-subagent-control.md` |
| F1-display per-kind one-line renderers | Message Display rows on `main` (f2, f4); old renderer WIP rejected (`reviews/f1display.md`); open: better first lines per block, golden tests | `streams/f1-display.md` |
| F1-core context provenance | A1+A2+B1+mixed sections on `main` (f2, f3); open: custom `user_message_template` prefixes stay `Whole` | `streams/f1-context-provenance.md` |
| F2/F3 F6/F7 sidebars wired into the view | on `main` (f4); open: F6/F7 hang on very large sessions (see stream file), dock glyph/hover polish | `streams/f2-f3-sidebars.md` |
| F6/F7 trees, removal, colours, saved panel settings (GRB-001 M12–M15) | on `main` (f3, f4), CI green 2026-10-19; open points in the stream file | `streams/f6-f7-trees.md` |

This table is the historical stream inventory, not the current delivery queue.
Use the [owner-takeover plan](plan/owner-takeover.md) for active ordering and gates.
GRB-001 remains the historical design owner for context-control proposals; this
inventory does not establish their current implementation or acceptance state.

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
| `grb/f1core` (`acc62327`, second cut) | mixed sections squashed onto `main` (`5c36f151`, 2026-10-20) |
| `grb/m11` (`6400f9b0`, second cut) | B2+B3+list/outcome squashed onto `main` (`33db3eb8`, 2026-10-20) |

## Archive branches (history only, do not build on them)
- `archive/pre-fold-main` — the complete pre-fold `main` history (2026-10-19, tip `3da4bf09`+docs);
  every pre-fold sha cited in the docs resolves here. Local only.
- `archive/f1core-wip-sections` — earlier F1-core attempt with `Sections::{Whole, Split}` on items
  and a legacy wire test; reference for the mixed-section increment.
- `archive/f1core-wip-48671d59` — first F1-core attempt (rejected: stored a derived category).
- `archive/m11-full-wip` — M11 stash from before a rebase (superset of experiments).
- `archive/testfix-attempts-501aa7f0`, `archive/testfix-attempts-final` — rejected test-fix attempts.
- `archive/thinking-review-prerebase-20260926` — M9 state before its rebase.
