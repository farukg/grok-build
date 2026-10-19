# F6/F7 trees, context removal and saved panel settings (GRB-001 M12–M15)

Branch: none; the work lands on `main` directly after `28c93489`.

## Goal
F6 (outline) and F7 (context sidebar) behave like tree views with documented modifier gestures. Every action they offer works or
names the concrete reason (context removal included). Rows carry colour and structure per kind. Panel settings are saved globally and
per session and come back after a restart. Design: `docs/fork/design/f6-f7.md` (F7 keys in §5, F6 keys in "F6 outline keys").

## State (2026-10-18)
Implemented; built and tested by CI only. Nothing was compiled or run locally. The `linux-build` workflow builds the release binary;
the manually started `linux-test` workflow runs the `xai-grok-pager` tests, the `xai-grok-shell` removal tests and the
`xai-chat-state` / `xai-grok-sampling-types` tests.

| Part | Where |
|---|---|
| Tree navigation and bulk gestures | `P/views/tree_nav.rs`, F6 `P/views/timeline_panel/` + `P/app/agent_view/timeline_panel/`, F7 `P/app/agent_view/context_sidebar/` |
| Removal with named reasons | shell `S/session/remove_items.rs`, `S/session/storage/remove_filter.rs`; pager `P/app/agent_view/removal_verdict.rs`, `P/app/dispatch/rewind_remove.rs`, `P/acp/turn_ledger.rs` |
| Row colour and structure | `P/views/row_style.rs` |
| Saved settings | `P/app/panel_settings/`, shared edit layer `P/app/agent_view/panel_persistence.rs` |

## What is new, what is hooked
The change is mostly new files. Numbers come from `git diff --numstat` and `git status` taken against `28c93489` before the commit;
the docs under `docs/fork/` are not counted.

- New: 42 files, 8101 lines (9 of them test files, 3879 lines).
- Existing upstream-owned files (present in the upstream base `4c72e282`): 19 files, +44 / −13 lines. These carry the hooks.
- Existing fork-owned files (created in the fork after the base, free to change): 15 files, +2653 / −415 lines.

### New files
| Path | Purpose |
|---|---|
| `.github/workflows/linux-test.yml` | manual workflow that runs the pager, shell removal, chat-state and sampling-types tests |
| `P/acp/turn_ledger.rs` | remembers which entry is followed by a hidden auto-wake turn, so a removed turn's rows stop there |
| `P/acp/turn_ledger_tests.rs` | tests of the ledger |
| `P/app/agent_view/context_sidebar/commands.rs` | F7 navigation, activation and bulk gestures; reasons a context switch cannot be sent |
| `P/app/agent_view/context_sidebar/footer.rs` | F7 footer rows (hint, save target, standing note) and their height |
| `P/app/agent_view/context_sidebar/gesture.rs` | maps F7 keys and clicks to commands |
| `P/app/agent_view/context_sidebar/lines.rs` | coloured F7 title and row lines with section summaries |
| `P/app/agent_view/context_sidebar/outline.rs` | F7 tree model: sections as branches, rows as leaves |
| `P/app/agent_view/context_sidebar/persistence.rs` | saves and restores F7 section folds and message-display forms |
| `P/app/agent_view/context_sidebar_persistence_tests.rs` | tests of F7 saving and restoring |
| `P/app/agent_view/panel_persistence.rs` | shared store handle, edit layer (`g`) and save/restore glue for F6 and F7 |
| `P/app/agent_view/removal_test_support.rs` | test helpers for numbered prompts and registered tool calls |
| `P/app/agent_view/removal_verdict.rs` | decides before sending whether a row can be removed and names the reason |
| `P/app/agent_view/removal_verdict_tests.rs` | tests of the verdict |
| `P/app/agent_view/timeline_panel/disclosure.rs` | applies tree steps, Space, bulk gestures and clicks to F6 folds |
| `P/app/agent_view/timeline_panel/footer.rs` | F6 footer context and cached removability of group rows |
| `P/app/agent_view/timeline_panel/persistence.rs` | saves and restores F6 turn folds and the turn default (`t`) |
| `P/app/agent_view/timeline_panel_persistence_tests.rs` | tests of F6 saving and restoring |
| `P/app/dispatch/rewind_remove_tests.rs` | tests of the removal input path and the completion handler |
| `P/app/panel_settings/codec.rs` | TOML names on disk, decoding and in-place encoding |
| `P/app/panel_settings/deviation.rs` | stores a wanted value as the smallest edit against the layer beneath |
| `P/app/panel_settings/error.rs` | load warnings, save errors and save outcomes |
| `P/app/panel_settings/layer_file.rs` | one layer's file: load, atomic owner-only save, backup of a damaged file |
| `P/app/panel_settings/mod.rs` | module doc and re-exports |
| `P/app/panel_settings/store.rs` | the two-layer store (global and attached session) |
| `P/app/panel_settings/store_tests.rs` | store tests |
| `P/app/panel_settings/turn_deviation.rs` | saves turn folds; drops the folds of rewound prompt numbers |
| `P/app/panel_settings/types.rs` | value types: layers, settings, keys, disclosure state, prompt ordinal |
| `P/scrollback/state/kind_default.rs` | unsets a kind's display default |
| `P/scrollback/state/selection/group_expansion.rs` | opens or closes several groups in one rebuild |
| `P/views/row_style.rs` | colour and structure per row class for F6 and F7, from theme fields only |
| `P/views/row_style_tests.rs` | tests of the row styles over several themes |
| `P/views/timeline_panel/disclosure.rs` | folded turns of the outline |
| `P/views/timeline_panel/footer_memo.rs` | memoised footer height and group-removability answers |
| `P/views/timeline_panel/gesture.rs` | F6 navigation keys, bulk gestures and click classes |
| `P/views/timeline_panel/hint.rs` | F6 footer hints packed to the column width, save-target line |
| `P/views/timeline_panel/outline_nav.rs` | F6 panel state: navigation, bulk planning, cursor repair, hit tests |
| `P/views/timeline_panel/outline_view.rs` | cached tree shape and visible rows of the outline |
| `P/views/timeline_panel/row_line.rs` | one styled outline row |
| `P/views/tree_nav.rs` | pure tree-navigation state machine over flat rows |
| `P/views/tree_nav_tests.rs` | tests of the state machine |
| `S/session/acp_session_tests/remove_items_tests.rs` | handler tests on a real session actor |

### Hooks in existing files
† = file created in the fork after the upstream base; the others are upstream-owned.

| Path | Lines added | Lines removed |
|---|---|---|
| `P/acp/mod.rs` | 1 | 0 |
| `P/acp/tracker.rs` | 3 | 0 |
| `P/actions/defaults.rs` | 2 | 2 |
| `P/app/agent_view/mod.rs` | 8 | 4 |
| `P/app/agent_view/session.rs` | 6 | 0 |
| `P/app/app_view.rs` | 3 | 0 |
| `P/app/app_view_tests.rs` | 1 | 0 |
| `P/app/dispatch/rewind.rs` | 1 | 0 |
| `P/app/dispatch/router.rs` | 2 | 6 |
| `P/app/dispatch/tests/mod.rs` | 1 | 0 |
| `P/app/event_loop.rs` | 1 | 0 |
| `P/app/mod.rs` | 1 | 0 |
| `P/scrollback/block.rs` | 4 | 0 |
| `P/scrollback/state/mod.rs` | 1 | 0 |
| `P/scrollback/state/selection.rs` | 2 | 0 |
| `P/views/mod.rs` | 2 | 0 |
| `S/session/acp_session.rs` | 3 | 0 |
| `S/session/mod.rs` | 1 | 1 |
| `S/session/persistence.rs` | 1 | 0 |
| † `P/app/agent_view/context_removal.rs` | 37 | 44 |
| † `P/app/agent_view/context_sidebar.rs` | 87 | 67 |
| † `P/app/agent_view/context_sidebar_tests.rs` | 556 | 0 |
| † `P/app/agent_view/sidebars.rs` | 23 | 6 |
| † `P/app/agent_view/timeline_panel.rs` | 33 | 34 |
| † `P/app/agent_view/timeline_panel_tests.rs` | 861 | 2 |
| † `P/app/dispatch/rewind_remove.rs` | 141 | 32 |
| † `P/app/effects/remove_context_items.rs` | 1 | 2 |
| † `P/views/sidebar/mod.rs` | 36 | 24 |
| † `P/views/sidebar/tests.rs` | 150 | 0 |
| † `P/views/timeline_panel.rs` | 117 | 81 |
| † `S/session/acp_session_impl/remove_items.rs` | 30 | 45 |
| † `S/session/remove_items.rs` | 96 | 1 |
| † `S/session/storage/remove_filter.rs` | 187 | 58 |
| † `S/session/storage/remove_filter_tests.rs` | 298 | 19 |

## Open points
- Nothing was run locally; the CI is the only compiler. `linux-build` (release build) is green at `51886e04`. The manually
  dispatched `linux-test` run 39183664639 at `51886e04` passes the new tests (pager 10443 passed). Its 5 failures are not
  from this work: 4 upstream doctor fixture tests (see STATUS.md) and `xai-chat-state`
  `actor::tests::apply_turn_request_pruning_is_noop_under_half_window` (crate unchanged). Neither workflow runs
  `clippy::indexing_slicing`.
- The rewind picker still lists removed turns (the shell removal does not touch `prompt_texts`).
- A removal does not invalidate a two-pass compaction prefire.
- The sealed-blob id of a removed `x_search` call is unverified (only `web_search` ids were seen).
- Terminals often do not deliver Shift+Enter, Ctrl+Enter or modified clicks; `+`, `-` and `*` reach the same targets. Ctrl+Enter is
  globally bound to `InterjectPrompt` and acts as a bulk gesture only while F6 or F7 has focus.
- After a restart that follows a removal, the next prompt can reuse the number of a removed prompt; a turn fold saved under that
  number would then apply to it.
- A cancel-with-rewind of a prompt the shell has not acknowledged does not clear saved turn folds (such a prompt holds no number).
- No test yet proves that a failed `ReplaceChatHistory` fails the next `FlushAndAck`.
- The `○` and `⊘` row markers of `ContextPresence` have no data source; F6 does not show markers yet.
