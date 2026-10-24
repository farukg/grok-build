# Shell caller wakeup — author-only result

PARTIAL; not verified. No builds, checks, tests, formatting, staging, publication or runtime state changes were performed. Existing delivery/Steer WIP remains intact. Read the prior delivery result and completed independent Shell review first; that review did not cover the new wakeup changes below.

## Evidence and reproduced source conditions

Read-only probe: `/tmp/grok-goal-4f7553a1dbe6/implementer/delivery-shell/observe-wakeup.py`. Captured output: `/home/fg/.grok/sessions/%2Fhome%2Ffg%2Fprojects/01a1641b-a389-7f60-b549-e78e4488946c/terminal/call_O68Jsn2MSxGVOLuHE62EaYXZ.log`.

Named parent: `01a14913-5ab0-7391-9209-617d5378af91`.
- `01a16422-9bda-7600-82b8-b5476b44365b`: current metadata completed; started 15:05:11Z, completed 15:08:11Z; parent history contains two spawned and two finished records.
- `01a1641d-5d34-7041-816a-96878327f31e`: current metadata completed; started 15:04:30Z, completed 15:09:43Z; parent history contains two spawned and two finished records.
- `01a16417-ffb0-7613-858a-ed153fd711d7`: current metadata running with newer start 16:34:10Z; history contains prior finished and subsequent spawned records.
- `01a1642b-8558-7ce3-93b1-fb14da7e642a`: current metadata running with newer start 16:05:40Z; history contains prior finished and subsequent spawned records.
- Current parent goal status is user_paused / Idle. This is NOT evidence of its goal gate at the earlier completion times.

These observations support completed historical attempts and later reuse. They do not prove historical parent admission, its feature flags, or running binary revision. No state was changed to manufacture a reproduction. Actual runtime reproduction/test execution is prohibited by the author-only contract.

## Confirmed causal source defects and changes

### Runtime error mistaken for intentional stop

Coordinator `finish_child` suppressed asynchronous surfacing for EVERY Interrupted state. Shell `AutoWakeInputs::from_completion` independently made the same suppression. Receipt/actor failures are represented as Interrupted { Error { message } }, so a genuine runtime failure could be buffered for later retrieval without admitting a parent wake.

Added one semantic classifier, `SubagentResult::permits_completion_wake`, in Tools task/types.rs. Completed, Failed and Interrupted(Error) can wake; Running and explicit cancellation/stop/pause/teardown/workflow/restart/orphan/limit causes do not. Coordinator surfacing and Shell wake eligibility now share that classifier. No failure is relabelled successful; error causes and explicit Stop/Cancel semantics are preserved.

### Active parent wake held behind whole-turn completion

`inject_subagent_completed_prompt` sends an ordinary protected synthetic Prompt with send_now=false. `maybe_start_running_task` refuses to promote while a parent task is running. Human follow-up Steer promotion does not apply to that synthetic runtime input. Tool completion reminders can independently surface completions after a tool, but a sampled parent without another tool can leave the admitted wake queued until its entire task ends.

Added `SessionActor::drain_subagent_wakes_at_safe_point` to the existing admitted-message safe-point reducer. It selects only a queued SubagentCompleted input distinct from the running input, reuses build_wake_turn_message's coordinator peek and reported ledger, projects ConversationItem::subagent_completed into the existing turn, then settles/removes that exact synthetic input. It does NOT blindly drain coordinator completion buffers, abort a tool, spawn a parallel turn, touch human/manual queue rows, or cancel children. It validates that the running/queued identities still match before projection. Duplicate already-reported wake inputs settle silently without another projection. Chat-state rejection logs an error and leaves the queued input available.

## Authored regressions

Shell `agent/subagent/tests/mod.rs`:
- `completion_errors_admit_parent_wake_but_explicit_stop_stays_silent`: Completed, Failed and Interrupted(Error) produce an actual injected parent Prompt; human explicit stop does not.

Shell `session/acp_session_tests/auto_wake_suppression_tests.rs`:
- `active_caller_projects_success_and_error_wakes_at_safe_point_once`: actual queue_input -> production admitted-message safe point, success/error content, existing running prompt preserved, original and retried receipts settled, no duplicate history projection, coordinator buffer is peeked rather than drained.
- `idle_caller_commits_success_and_error_wakes_without_a_human_prompt`: handle_prompt/wake digest reaches persistence-ack commit, without a new human prompt. Uses existing actor fixture/fake coordinator; not a process/binary E2E test.
- `restored_reported_ledger_suppresses_retried_completion_wake`: serialized registered ReportedTaskCompletions is restored into a fresh actor, and a replayed queued wake produces no second projection. This is resource-rehydration coverage, NOT crash-atomic process restart evidence.

Tools `task/coordinator_tests.rs`:
- `finishing_parent_turn_drain_keeps_independent_background_child_running`: real coordinator usage-drain protocol resolves while independent background child remains Running and can later complete normally.

Previously authored opposite-direction integration remains `agent/subagent/tests/wake.rs::{live_steers_reach_next_sample_in_order_before_parent_completion,live_steers_wait_for_active_tool_then_reach_next_sample_before_parent_completion}`. Those exercise coordinator -> ShellChildRuntime -> SessionActor -> mock Responses sampler, active tool gating, ordered unique messages, child gateway/history and final parent result. They remain unexecuted. Existing receipt Held/Auto, caller-abandonment and terminal/persistence-race cases are preserved.

## Exact changed paths for this wakeup unit

Root `/home/fg/projects/org/forks/grok-build/`:
- crates/codegen/xai-grok-tools/src/implementations/grok_build/task/types.rs
- crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator.rs
- crates/codegen/xai-grok-tools/src/implementations/grok_build/task/coordinator_tests.rs
- crates/codegen/xai-grok-shell/src/agent/subagent/spawn.rs
- crates/codegen/xai-grok-shell/src/agent/subagent/tests/mod.rs
- crates/codegen/xai-grok-shell/src/session/acp_session_impl/reminders.rs
- crates/codegen/xai-grok-shell/src/session/acp_session_impl/interjection.rs
- crates/codegen/xai-grok-shell/src/session/acp_session_tests/auto_wake_suppression_tests.rs

No Pager, sig-orch, codex-rs, ChatState or persistence owner source was edited. No concrete concurrent shared-owner writer was observed; no ownership conflict is asserted.

## Remaining boundaries / full outstanding list

1. Goal-specific suppression remains a separate existing policy: AutoWakeInputs/InjectParams and build_wake_turn_message can suppress/discard subagent wakes under goal_loop_active. TaskCompletionReminder also marks them reported without surfacing when goal mode is active. The new safe-point method does not override that policy. Current user_paused observation cannot establish the gate's earlier value. This policy still needs explicit reconciliation with the expanded goal-direction requirement, not an unscoped removal of all goal/task Stop suppression.
2. Strict exactly-once across a PROCESS CRASH is NOT implemented or proved. Registered ReportedTaskCompletions is serialized, but chat-history projection and reported-resource persistence are separate commit owners. A crash between projection and ledger save, and reuse of one subagent id across attempts, require an attempt-keyed transactional completion admission/outbox and recovery contract. The resource-rehydration test does not close that gap.
3. No automatic replay/recovery of an unadmitted completed child into a restarted idle parent is added. Existing wake code relies on live coordinator buffers; durable child metadata alone does not reconstruct that buffer here.
4. New parent-direction tests stop at actor projection/persistence ack; real coordinator -> completed/error child -> idle/active parent -> mock sampler E2E (including active parent tool boundary and process restart) is still missing. Existing opposite-direction sampler tests are not substitute evidence.
5. Lead must compile/format/lint and run all new and existing wake/receipt/queue/cancel/replay tests. New tests have not established green evidence or deployed behavior. Runtime selected PATH binary and a running leader must be identified independently.

## Parent coordination / decision required

Please assign the attempt-keyed crash-atomic completion outbox/recovery contract with the ChatState/resource-persistence owner before claiming exactly-once process-restart delivery. Resolve goal-specific suppression explicitly for subagent results versus ordinary late background-task completions, preserving real human Stop/Cancel. The eight paths above contain the scoped, authored fixes; the remaining list is complete and is not presented as finished work.
