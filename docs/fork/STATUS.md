# Fork status

As of 2026-09-28 (W2 landed). Upstream base: `4c72e282` (`origin/main` = `xai-org/grok-build`).
`main` of this fork = upstream + the commits below. `main` is the integration branch:
everything that compiles and passes its tests lands here.

## On main (all built and tested)

| Area | Commits (oldest first) |
|---|---|
| M7 user prompts collapse to one line | `497ed2b8` |
| M8 subagent tokens in task rows | `1a8ac47b` |
| M10 goal planner: queue instead of abort, visible fork failure, forks keep summary | `85d93bb9`, `5c526c96`, `29756c6c` |
| M9 real thinking levels and served model (requested → served) | `2c50adbc`, `f4bb59eb` |
| M1/M3 session switcher everywhere, deterministic subagent tree navigation, one row-to-child open path | `191ec68d`, `6ac8fcdc`, `b695fdc9` |
| M4 subagent resume in every state | `d41e739b`, `fd27fedd`, `d6831313` |
| M2 timeline outline over every entry, expandable F6 panel with scroll sync and item actions | `117cea2a`, `43d0abc2`, `5ff9bce7`, `cc828a4c`, `17a90248` |
| Feedback tool gated by `features.feedback` | `7d9ff33e` |
| `/compact` "Send now" runs a queued agent command | `c8b1f15a` |
| Sibling cycling of subagents | `9d963560`, `2dbabb72` |
| Shared sidebar component + three-column geometry (not wired into the view yet) | `d13b0729`, `f6b45711` |
| Typed context policy (`ContextCategory`, `ContextPolicy`) and whole-item classifier `ConversationItem::context_category()` | `595f660d`, `3b54b425` |
| Typed message display defaults (`MessageKind`, `DisplayForm`, `DisplayDefaults`, `EntryForm`) | `c8edd38f`, `d3b9b796`, `292a78cc`, `3ca635a9`, `f8bdaf94`, `8473b818`, `c5c2c850` |
| Shell: child session hosts in the registry | `24524633` |
| M11 typed subagent interruption state (types only) | `b540b07e` |
| M5 remove turns / tool exchanges from the model context via the timeline, persisted across replay | `5f39a22f`, `3d97d8ef`, `e4a3117b`, `b6f08d05`, `b043352b` |
| W2 subagent view = normal session view (child sessions are top-level views, takeover modal removed) | `a632eab` |

Last full test run of `xai-grok-pager`, `xai-grok-shell`, `xai-grok-sampling-types` on `c5c2c850`:
green except the known failures below. `3b54b425` passed `cargo test -p xai-grok-sampling-types`.
`a632eab` (W2): `cargo test -p xai-grok-pager --no-fail-fast` green except the known failures;
`cargo clippy -p xai-grok-pager -p xai-grok-shell --tests` without errors.

### Known failures (not regressions)
- Upstream, also failing on `4c72e282`: `diagnostics::doctor_format::tests::limited_color_output_is_stable`,
  `doctor_cmd::tests::{human_mixed_fixture_is_exact, json_contract_is_structural_stable_ordered_and_ansi_free, json_empty_fixture_pins_null_policy}`
  (fixtures expect 6 themes, the tree has 7).
- Flaky: `app::acp_handler::tests::settings::settings_update_clearing_group_tool_verbs_reverts_to_default`.
  Appearance caches (`crate::appearance::cache::*`) are thread-local and seed from the developer's
  `[ui]` config on first read; tests that depend on them must set the value they need first.

## Work streams

| Stream | Branch | State | Stream file |
|---|---|---|---|
| W2 subagent view = normal session view | on `main` (`a632eab`) | done; follow-up W3 with W1-S | `streams/w2-subagent-session-view.md` |
| W1-S shell: children are first-class ACP sessions | `grb/w1s` | partial, review open | `streams/w1s-child-sessions-shell.md` |
| M11 agent + human control over subagents | `grb/m11` | partial, uncompiled wiring | `streams/m11-subagent-control.md` |
| F1-display per-kind one-line renderers | `grb/f1display` | partial | `streams/f1-display.md` |
| F1-core context provenance producers | new branch from `main` | brief ready | `streams/f1-context-provenance.md` |
| F2/F3 F6/F7 sidebars wired into the view | later | blocked on F1 (W2 is on `main`) | `streams/f2-f3-sidebars.md` |

Suggested order: W1-S and F1-core first (they unblock W3 and F2/F3), in parallel M11, F1-display.
Each stream merges `main` into its branch when needed (no rebases of pushed branches). A stream is
done only when `main` contains its progress: once green, squash it onto `main` and push (no PR).

## Archive branches (history only, do not build on them)
- `archive/f1core-wip-sections` — earlier F1-core attempt with `Sections::{Whole, Split}` on items
  and a legacy wire test; reference for the mixed-section increment.
- `archive/f1core-wip-48671d59` — first F1-core attempt (rejected: stored a derived category).
- `archive/m11-full-wip` — M11 stash from before a rebase (superset of experiments).
- `archive/testfix-attempts-501aa7f0`, `archive/testfix-attempts-final` — rejected test-fix attempts.
- `archive/thinking-review-prerebase-20260926` — M9 state before its rebase.
