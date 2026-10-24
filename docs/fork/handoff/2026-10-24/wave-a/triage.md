# Wave A CI-failure triage — commit bc85ceb9

Inputs: `../ci-bc85-runtime/diagnostics.json` (27 failed + 2 hanging), capture
`../ci-c8-fixtures/bc85-failed-ci.log`, STATUS.md known failures, runtime lane
`worktrees.d/org/forks/grok-build@ci-c8-format` (checkpoint 14a5f56c), canonical
lane `org/forks/grok-build` (checkpoint 074f54b5).

**Capture caveat:** the shell-lib step was killed (exit 143, job timeout, log lines
9770–9774) before cargo printed the per-test `failures:` stdout blocks. Only three
panics are visible in the capture (child-compaction NotTerminal ×2 runs, PTY
timeout). All other shell failures are classified from source + lane diffs.

## Bucket counts

- env_failures: 7
- addressed: 20 (19 failed + the hanging `active_caller…`; the PTY test appears in
  both failed and hanging lists and is counted once)
- open_items: 1 (`execute_tool_calls_records_the_product_row_and_execution_span`)

## env_failures (STATUS.md "Known failures", cloud container)

STATUS.md lines 70–72: four `agent::config::tests` api_backend/citation cases,
`session::acp_session::mcp_file_input::tests::*` (3),
`session::unified_list::tests::parse_list_req_forces_kind_under_process_chat_mode_only`.

1. `agent::config::tests::resolve_model_list_inherits_agent_type_and_api_backend`
2. `agent::config::tests::slug_propagation_inherits_api_backend_but_not_agent_type`
3. `agent::config::tests::slug_propagation_inherits_api_backend_into_config_entry_without_one`
   (the 4th STATUS case — citation — is not in the bc85 diagnostics; it passed here)
4. `session::acp_session::mcp_file_input::tests::file_advertisement_requires_host_preparation_and_bounded_filesystem`
5. `session::acp_session::mcp_file_input::tests::finalized_randomized_wrapper_preserves_canonical_file_keys`
6. `session::acp_session::mcp_file_input::tests::messages_backend_advertises_inline_only_use_tool`
7. `session::unified_list::tests::parse_list_req_forces_kind_under_process_chat_mode_only`

## addressed

### Wake lane — runtime checkpoint 14a5f56c (typed wakes)

All 11 failed auto-wake tests plus the hanging `active_caller_projects_success_
and_error_wakes_at_safe_point_once` are rewired by the runtime lane's typed-wake
rewrite of `crates/codegen/xai-grok-shell/src/session/acp_session_tests/auto_wake_suppression_tests.rs`
(diff 14a5f56c, 305 changed lines):

- prompt-id string matching (`PromptOrigin::from_prompt_id("subagent-completed-x")`)
  replaced by typed `PromptOrigin::SubagentCompleted { completion }` with bound
  `attempt_id` — wake_request helper, diff hunk @@ -26,6 +28,25.
- `run_wake_turn` migrated from the old 11-arg `handle_prompt` to
  `handle_turn_input(TurnInputRequest{…})` wrapped in a 5s timeout — the missing
  deadline is the plausible cause of the bc85 hang of `active_caller…` (also given
  its own per-iteration timeout, @@ -154,10 +185,7).
- `subagent_completed_drops_matching_pending_input` gains running-task stubbing and
  a full commit-oracle (`commit_observed_subagent_tool_result` binding the observed
  attempt, receipt persistence) — @@ -1016,8 +1042,10.
- `already_reported_wake_ends_turn_without_sampling`, `idle_caller_commits…`,
  `preempted_wake_is_redelivered…`, `promotion_drops_reported_wake…`,
  `refused_wake_leaves_completion_unreported`, `wake_missing_from_buffer_falls_back…`,
  `wake_turn_digest_coalesces…`, `wake_turn_under_goal_loop_projects…` all migrate to
  `wake_request`/typed origins (diff hunks @@ -1567, @@ -1616, @@ -1625, @@ -1640, …).
- `shutdown_drops_pending_synthetic_inputs` is RENAMED to
  `shutdown_drops_pending_ephemeral_inputs` and its semantics change: SubagentCompleted
  is no longer dropped on shutdown (@@ -1223,7 +1280,7).

### Wake lane — compatibility notes for merge authors (both checkpoints touch it)

- **Certain textual conflict:** both lanes prepend different `#[path]` module lines to
  `auto_wake_suppression_tests.rs` — runtime adds
  `#[path = "completion_queue_lifecycle_tests.rs"] mod queue_lifecycle;`, canonical
  (074f54b5) adds `#[path = "auto_wake_fixtures.rs"] mod fixtures; use fixtures::*;`.
  Merge must keep both.
- **Divergent fixture shapes:** canonical's `auto_wake_fixtures.rs` builds
  `TaskWakeAdmission` + `TaskWakeFallback` (oneshot-acked queue admission, no attempt
  id in prompt_id); runtime's `wake_request` builds `QueueInputRequest::from_input_origin`
  with prompt_id `"subagent-completed-{id}/{attempt_id}"` and `verbatim: true`.
  Canonical's fixture `task_wake_admission` does not encode the attempt id the runtime
  oracles now assert on (`completion.id.attempt_id`); if canonical's admission path
  regenerates prompt_ids without the attempt suffix, runtime's dedup/oracle assertions
  (e.g. `assert_eq!(persisted.receipts[0].completion, completion.id)`) will not hold.
  The runtime lane also depends on `QueueInputRequest::from_input_origin`
  (`acp_session_impl/prompt_queue.rs:59`) and `handle_turn_input`
  (`acp_session_impl/turn.rs:493`) — both exist in canonical main, so the API surface
  merges.
- **Shutdown semantics:** runtime's rename assumes SubagentCompleted survives shutdown;
  canonical 074f54b5 also touches `session/acp_session.rs` (+3) for admission. Confirm
  the merged shutdown path agrees before accepting the rename.
- Both lanes also edit `agent/subagent/tests/mod.rs` and `rest.rs` (canonical 204/64
  changed lines, runtime 64/30) — expect conflicts; canonical introduces
  `completion_fixtures.rs` (`DurableCompletionFixture`), runtime edits the same tests
  with different helpers.

### Non-wake failures

- `agent::mvp_agent::tests::child_prompt_tests::finished_coordinator_child_compacts_
  persisteded_checkpoint_without_parent_delivery` — bc85 panic at
  `child_prompt_tests.rs:518:10`: `NotTerminal { subagent_id }` from
  `durable_resume_source_at` (`agent/subagent/mod.rs:1615`, meta.json still `Running`
  when read after the spawn result resolved). Canonical 074f54b5 reworks terminal
  publication ordering: `coordinator_state.rs` `TerminalPublicationState`
  Committing→Publishing→Published with attempt-bound `TerminalPublication` guards and
  `coordinator/completion.rs` (+35) — the spawn result should no longer resolve before
  the terminal meta write. Confidence: medium (no build acceptance in either
  checkpoint; canonical also carries `completion_fixtures.rs` with
  `completion_data_with_initial_meta` proving the commit-before-result invariant).
- `send_subagent_message_row_opens_child::send_subagent_message_row_opens_child`
  (failed + hanging) — bc85 panic `Enter did not open the takeover: timed out after 60s
  waiting for screen text "q/Esc:back"` (capture line 2410). Runtime lane rewrote the
  test (diff 14a5f56c + commit 8e3f9ee1 "prove PTY focus with actual selection
  transitions"): it now waits on `PARENT_NAVIGATION` via `assert_child_open`
  (`pty_e2e/send_subagent_message_row_opens_child.rs:14-22`) instead of the absent
  `q/Esc:back` hint, with focus/selection-transition verification in
  `child_session_navigation.rs` (+38). Addressed by runtime lane.
- `session::acp_session::prompt_mode_transition_tests::a_synthetic_turn_inherits_plan_
  mode_instead_of_ending_it` — runtime lane migrated the test from
  `PromptOrigin::from_prompt_id` strings to typed `PromptOrigin` variants incl.
  attempt-bound `SubagentCompleted` (diff 14a5f56c,
  `acp_session_tests/prompt_mode_transition_tests.rs` @@ -192,31 +192,43). Same
  typed-wake root cause; addressed.
- `session::persistence::cleanup_stale_sessions_tests::*` (6 tests: active_session_
  keeps_backdated_write_once_artifacts, active_session_sweeps_only_backdated_blob_files,
  dot_entries_are_kept…, emptied_cwd_dir_is_removed…, idle_session_is_removed_whole,
  mark_session_live_keeps…) — runtime lane fixes the shared fixture
  `write_session` to write a real `Summary::new(...)` JSON with explicit mtime instead
  of an empty `summary.json` (diff 14a5f56c,
  `session/persistence_cleanup_stale_sessions_tests.rs` @@ -25,7 +25,18), matching a
  sweep that now parses summary.json (admission recovery, cf. new test
  `malformed_summary_keeps_stale_session_for_admission_recovery` @@ -286,3 +297,26).
  Addressed by runtime lane.

## open_items

- `session::acp_session::tool_call_telemetry_tests::execute_tool_calls_records_the_
  product_row_and_execution_span` — **no individual stdout in the capture** (step
  killed at exit 143 before failure blocks were printed; only the FAILED marker at
  capture line 6671). Neither checkpoint touches telemetry sources (no telemetry
  files in either 14a5f56c or 074f54b5 diffstat); the runtime checkpoint message
  itself says "Telemetry … still require actual diagnostics". Source analysis
  (`acp_session_tests/tool_call_telemetry_tests.rs:420-614`):
  1. Most plausible: missing/late product row — `product_event` panics
     "no product row for {invocation_id}" (test line 300) after
     `xai_grok_telemetry::session_ctx::drain_pending(5s)` (lines 480, 561). The
     sibling test `read_profile_agrees…` (serial, same product server pattern)
     passed at bc85, so the delta is this test's two-phase mode switch
     (`init_product(SessionMetrics)` → asserts no product row for `off-hit`
     (lines 496-501) → `init_product(Enabled)`): a global `xai_grok_telemetry::init`
     re-init racing pending events from the off-phase, or SessionMetrics mode now
     emitting `grok-shell-tool_call_completed` rows, breaks exactly the line
     496-501 "no row in SessionMetrics" oracle.
  2. Secondary: tool-identity assertion `field(off, "tool_id") == Some("GrokBuild:grep")`
     (line 488) depends on `FinalizedToolset::registered_identity("search_code")`
     honoring the `.with_name` override (`xai-grok-tools/src/registry/types.rs:1488`);
     a finalize-path change that registers the alias under a different client_name
     degrades to opaque and fails lines 488/492 first.
  Needed next step: rerun `-p xai-grok-shell --lib tool_call_telemetry` locally (or in
  CI) to capture the actual assertion before fixing; both checkpoints explicitly
  defer this.

## Hanging tests

- `send_subagent_message_row_opens_child` — see addressed (PTY, runtime lane).
- `session::acp_session::auto_wake_suppression_tests::active_caller_projects_success_
  and_error_wakes_at_safe_point_once` — see addressed; runtime lane wraps every
  iteration in a 5s timeout and migrates to `wake_request` with an acknowledged
  receipt oracle (diff @@ -154,10 +185,7).
