# Wave A merge-fidelity refutation report — 2026-10-24

Worktree: /home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover (grb/owner-takeover, HEAD 14a5f56c)
Baselines: /home/fg/projects/tmp.d/grok-wave-a/baseline/{canonical,runtime}/<repo-relative-path>

## Verdict: REFUTED (minor) — 1 substantive dropped-coverage finding, 2 nits

One hunk-level assertion vanished beyond the four sanctioned deviations. No production
semantics, phase oracle, or API wiring was dropped or weakened.

## Per-file findings

### 1. mvp_agent/tests/subagent_spawn_context_tests.rs — CLEAN
Byte-identical to both exported sides (empty diff vs canonical and vs runtime).
Sanctioned deviation (a) verified: no Arc-gate test present anywhere in the merged file
and both baselines already agree on its absence.

### 2. subagent/tests/mod.rs — CLEAN modulo sanctioned (b)(c)(d)
Diff vs canonical = 536 lines, vs runtime = 71 lines; every canonical-only hunk was
relocated or strengthened, not dropped:
- `auto_wake_test_request`, `pending_shell_completion_fixture`, `running_meta_json`
  (sanctioned (d): gone everywhere, zero users remain) → moved to
  agent/subagent/completion_test_support.rs:15,37 (sanctioned (c)).
- `DurableCompletionFixture`/`completion_data_with_initial_meta`: single source at
  tests/completion_fixture.rs:5,20, reached via `#[path]` re-export in
  completion_test_support.rs:8-12; declared exactly once. tests/completion_fixtures.rs
  still on disk (git status `A`) with its mod/import gone from mod.rs — matches
  sanctioned (c), pending `git rm`.
- Canonical's `wake-error-class` test was not dropped: re-expressed stronger
  (real CoordinatorFixture backend, real storage, metadata status/error table, FIFO
  parent observation) in mod.rs merged region.
- Canonical's inject_subagent_completed_prompt scheduler-hint test retained:
  mod.rs:880-926 (`=== Output ===`, `renamed_scheduler_delete("loop-123")`,
  `block < cleanup` ordering all present).
- Sanctioned (b) verified: post-admission `initial_metadata` byte-equality assertion
  removed; replaced by producer-ack-then-terminal-metadata expectations.
- Runtime's `assert_eq!(live, 1)` raw-transport count became `live.len() > 1` with
  identical-`eventId` and persisted-count==1 assertions (mod.rs:2684-2706) — a
  deliberate at-least-once reframe, not a softening; persisted exactly-once is still
  asserted.

### 3. subagent/tests/rest.rs — CLEAN
Merged is a strict superset of the runtime side (zero dropped lines vs runtime).
Vs canonical: adds `snapshot_ref_write_promotes_nonterminal_status_to_terminal`
(rest.rs:508-617) and strengthens `snapshot_ref_write_requires_a_producer_commit_…`
with commit/publish/replay idempotence checks. No canonical assertion lost except the
sanctioned (b) byte assertion.

### 4. session/completion_delivery.rs — CLEAN
Byte-identical to the canonical side. Zero unwrap/expect/panic; `result_reference`
match is exhaustive over CompletionObligation; reference text is derived at read time
via `model_text`/`completion_reference_text` instead of stored pre-concatenated text
(no stored derivable value).

### 5. agent_targets_tests.rs — 1 nit
Runtime's direct `ChildRunOutput` construction was replaced by canonical's
`finish_child_with_result`/`finish_child_with_result_before_visibility` helpers
(semantics preserved; helpers resolve in active_message::tests). Canonical's missing
publication-drain step was added (agent_targets_tests.rs:572-576) via
`terminal_published_rx`/`handle_terminal_publication`, which resolve in the merged tree.
- **Nit (formatting):** agent_targets_tests.rs:13-16 — merge artifact leaves
  `finish_child_with_result,` starting at column 0 inside the import block; file has
  no `rustfmt::skip` attr, so this is a non-rustfmt-compliant merged hunk.

### 6. auto_wake_suppression_tests.rs — 1 SUBSTANTIVE finding, 1 nit
Canonical-only test names (`shutdown_drops_pending_synthetic_inputs`,
`preempted_wake_is_redelivered_by_next_between_turn_drain`) are renamed but their
semantics survive: → `shutdown_drops_pending_ephemeral_inputs` (line 1289) and
`queued_child_survives_user_priority_until_next_between_turn_drain` (line 1662, with
"exactly one digest" assertion at 1734). Runtime's `queued == ["subagent-completed-sa-live/at1.1"]`
assertion is re-expressed at line 1650 via `expected_live_prompt_id`. Helpers
(`task_wake_admission`, `task_output_result`) resolve in auto_wake_fixtures.rs:3,42 and
acp_session_tests/support.rs:1092.

- **SUBSTANTIVE (dropped assertion, not sanctioned):** canonical's shutdown test pushed a
  `SubagentCompleted`-origin pending input and asserted it was dropped, naming
  `SubagentCompleted` in the failure message. The merged
  `shutdown_drops_pending_ephemeral_inputs` (auto_wake_suppression_tests.rs:1289-1338)
  covers only TaskCompleted / NotificationDrain / GoalSummary origins — the
  SubagentCompleted-origin drop case is no longer proven anywhere. Production still
  drops it (`session/mod.rs` PromptOrigin::policy → SubagentCompleted not
  ShutdownPolicy::Drain; acp_session_impl/queue_mutation.rs:30-32
  `is_preemptible_runtime_wake`; acp_session_impl/tool_calls.rs:2915
  `drop_pending_synthetic_items`). Fix: add one `subagent_completed_input("sub-1")`
  row to the queue in that test and restore `SubagentCompleted` to the assertion message.
- **Nit (comment accuracy):** subagent/tests/mod.rs:2692 claims "the real AppView replay
  test owns visible exactly-once" — no test by that name exists in the tree; the nearest
  actual owner is rest.rs:2005 `reconcile_dedups_replay_and_running_meta_sources` plus the
  persisted-count assertions in the same test. Comment names a non-existent test.

## Phase oracles (ground 2) — VERIFIED SOUND
- rest.rs:508-617 (`snapshot_ref_write_promotes_nonterminal_status_to_terminal`): pins
  Running status + `completed_at.is_none()` + byte-stable meta.json before commit; real
  storage (`fixture.storage.load_session`) shows no SubagentFinished; then
  `prepare_completion_admission` + `prepared_producer()` with full terminal expectations
  (state, completed_at, duration_ms=42, tool_calls=3, turns=1, error=None); meta.json
  byte-identical after `publish_committed_completion` and after a second publish (replay),
  persisted SubagentFinished count == 1.
- mod.rs:2296-2370 (`promote-cancel`): prepare → publish → persisted count == 1, live
  notification count == 1. No value softened vs either side.
- mod.rs:2447-2562 (`unproven-exit`) and mod.rs:2564-2710 (`startup_admission_timeout_…`
  "promote-timeout"): full before-commit running-metadata pin, producer event state ==
  `Failed { "Subagent initial prompt was not admitted before the deadline" }`, terminal
  metadata table (status/completed_at/duration/tool_calls/turns/error), byte-stability
  after publish and replay, persisted count == 1.

## API coherence (ground 3) — VERIFIED SOUND
- `test_gateway_with_receiver` resolves: test_support/lsp_runtime.rs:14.
- `SubagentState` (crates/common/xai-tool-types/src/subagent_state.rs:33) uses struct
  payloads (`Failed { message }`, `Interrupted { cause }`) as used in merged tests.
- `CompletionWake`/`CompletionWakeKind`/`CompletionWakePolicy` shapes consistent between
  auto_wake_suppression_tests.rs:25-32 and completion_delivery.rs:383-398.
- `pending_shell_completion_fixture` defined exactly once (completion_test_support.rs:37);
  `DurableCompletionFixture` single source (tests/completion_fixture.rs:20).
- No conflict markers remain in any of the six files.

## Code rules (ground 4) — VERIFIED SOUND except the two nits above
Production file (completion_delivery.rs): no unwrap/expect/panic, exhaustive matches,
derived-not-stored text, enums carry payloads. Deviations: import formatting nit
(file 5) and misleading comment nit (mod.rs:2692); no drive-by reformats elsewhere.
