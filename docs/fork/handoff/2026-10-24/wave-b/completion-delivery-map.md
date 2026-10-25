# Completion/delivery wake map — gb-wake-1 + gb-wake-2

Read-only defect map of `worktrees.d/org/forks/grok-build@owner-takeover` (branch `grb/owner-takeover`).
All paths below relative to `crates/codegen/xai-grok-shell/src/` unless absolute.

## 1. Completion wake path (subagent → parent turn)

Hop chain (file:line):

1. Child completes; coordinator classifies. `agent/subagent/spawn.rs:482-509` `will_wake_for` /
   `agent/subagent/completion_publication.rs:8-44` `completion_event` → policy
   `Automatic` iff `completion_data.auto_wake_enabled` (captured at spawn from
   `SubagentSpawnContext.auto_wake_enabled`, `agent/subagent/mod.rs:586`; default `true`,
   `tools/tool_context.rs:349`).
2. `agent/subagent/spawn.rs:619-660` `enqueue_completion_wake` sends
   `SessionCommand::SubagentCompletionWake` with `PromptOrigin::SubagentCompleted { completion }`
   (`CompletionWakeKind::Fresh`, `session/completion_delivery.rs:390-410`).
3. `session/acp_session_impl/run_loop.rs:725-733` queues it via `queue_input` **unconditionally —
   no goal-status, no suppression check** — then calls `maybe_start_running_task`.
4. `session/acp_session_impl/prompt_queue.rs:113` `queue_input` appends to `pending_inputs`
   (human prompts may sweep preemptible synthetics first, `:230-250`).
5. Promotion: `session/acp_session_impl/notification_drain.rs:119` `maybe_start_running_task`;
   stale-sweep at `:240-262` drops a `SubagentCompleted` front only if its receipt already exists.

### Goal status vs the wake

- **Active**: per ADR-001 the subagent-only goal gate was deleted; wake is admissible at safe
  points (`completion_admission.rs:224` `drain_subagent_wakes_at_safe_point`). Goal-loop
  suppression remains only for *notifications* (monitors/tasks), `notification_drain.rs:530-540`
  (drops while goal Active **or Complete** when harness enabled).
- **UserPaused (via `/goal pause`)**: nothing on the subagent path reads goal status — the wake
  queues and promotes normally. Paused alone is not the blocker.
- **UserPaused (via Ctrl+C StopGesture)**: `session/acp_session_impl/cancel.rs:413-416` arms the
  wake barrier on `CancelKind::StopGesture`: `arm_wake_barrier` (`cancel.rs:97-112`) sets
  `task_wake_suppressed` gate + `state.notifications_suppressed = true` and persists
  `CompletionWakePolicy::HumanStopped` (`completion_admission.rs:88-99`, restored on restart via
  `run_loop.rs:1385-1389` / `SessionCommand::RestoreCompletionWakePolicy`). The queued wake is
  then **parked, not dropped**: `notification_drain.rs:216-228` reorders only a human-intent front
  to the queue head and returns — synthetic wakes sit in `pending_inputs` indefinitely;
  `completion_admission.rs:224-231` and `:281-286` (`drain_buffered_subagent_completions`) also
  early-return while the gate is set.

**This is the gb-wake-1 pause-drop site** (park/defer, not literal drop):
`notification_drain.rs:216-228` (promotion deferral) armed by `cancel.rs:97-112`.

### Re-delivery on resume

The gate clears only on human re-engagement: `run_loop.rs:760-770` (non-synthetic `Prompt`) and
`notification_drain.rs:383-390` (human front promotion), plus cancel paths at `cancel.rs:156-170,
324, 538, 578`. `resume_goal` (`session/acp_session_impl/goal.rs:874-1010`) **never** clears the
barrier, never reorders `pending_inputs`, and never calls `maybe_start_running_task` for parked
wakes — but since `/goal resume` is itself a human prompt it transitively clears the gate; the
parked wake is then only incidentally promoted behind whatever the resume continuation queues.
There is no explicit "re-deliver parked wakes on resume" step; delivery after resume is an
emergent side effect of queue ordering, and after a crash-restart the persisted
`HumanStopped` policy keeps suppression armed (`run_loop.rs:1385-1389`).

## 2. Stop-hook / turn-end contract (gb-wake-2)

- Turn end runs the Stop/SubagentStop gate: `session/acp_session_impl/stop_gate.rs:232-332`
  `run_stop_gate`. Decision inputs include a background-work snapshot
  (`stop_gate.rs:151-184`: outstanding bash/monitor tasks + active subagents + scheduled crons).
- **If no hooks are registered for `Stop`, the gate returns `AllowStop` immediately**
  (`stop_gate.rs:246-248`). There is no built-in "hold the turn open while children run": the
  session simply goes idle and relies on the synthetic-wake path (§1) for continuation.
  `StopGateDecision::KeepWorking` exists only as a hook-driven continuation
  (`stop_gate.rs:329-332`, cap 8 per turn at `:247-260`).
- The gate never reads goal status; the only goal-aware idle logic is the notification
  suppression at `notification_drain.rs:530-540`.
- A "wait" state exists only implicitly: `tool_context.blocking_wait_depth`
  (`prompt_queue.rs:317, 324-329`) drives *send-now steering* (auto interject into a blocked
  turn), it is not a model-facing wait contract, and nothing documents to the agent that ending
  its turn with live children parks the session until a wake or human input.
- "Any new input wakes it": true at the queue level for human prompts (they promote, clear the
  gate, `run_loop.rs:760-770`), but false for runtime wakes while the barrier is armed, and the
  whole contract is undocumented for the model (no system-prompt/reminder owner states it).

## 3. Workflow / task / monitor wake paths — separate admissions

- **Workflow**: separate command `SessionCommand::WorkflowCompletionTurn`,
  `run_loop.rs:2198-2258`. Admission at `:2199-2207`: `wake_suppressed = notifications_suppressed
  || goal_loop_active() || task_wake_suppressed gate`. If suppressed → `continue` — the command is
  **dropped, not parked** (only `is_unreported_completion` keeps state; no re-delivery on resume
  other than restart/reminders paths). Paused goals do not suppress it (`goal_loop_active` is
  Active-only, `session/acp_session_impl/goal_support.rs:1037-1041`), but a Ctrl+C barrier does,
  with loss of the wake.
- **Bash tasks (`TaskCompleted`)**: `admit_task_completion_wake`, `run_loop.rs:136-176` — rejected
  into a `TaskWakeFallback` when gate or `notifications_suppressed` set (fallback replayed later,
  `notification_drain.rs:42`).
- **Monitors/scheduler notifications**: `maybe_drain_notifications`,
  `notification_drain.rs:511-600` — dropped entirely while goal Active/Complete (harness enabled)
  or when suppressed; different admission from both above.

So there are **three distinct suppression/admission mechanisms** with different drop-vs-park
semantics for the same observable symptom.

## 4. Already fixed by the merged checkpoint work vs open

Fixed (per ADR-001/002, `completion_delivery.rs`, `completion_publication.rs`,
`completion_admission.rs`): attempt-bound `(subagent_id, attempt_id)` identity end-to-end;
durable projection receipts (`CompletionProjectionAck`, `commit_subagent_projection`
`completion_admission.rs:112-145`); goal-loop suppression of *subagent results* deleted; stale-wake
sweep by receipt; crash-atomic publication/recovery; `x.ai/session/delivery` human-delivery
receipts.

Open for these two symptoms:

1. **gb-wake-1**: the Ctrl+C `arm_wake_barrier` (`cancel.rs:97-112`) conflates "interrupt this
   turn" with "suppress all future completion wakes", and resume (`resume_goal`) has no wake
   re-admission step; parked subagent wakes rely on incidental queue ordering, workflow wakes are
   lost outright, and the persisted `HumanStopped` policy survives restarts.
2. **gb-wake-2**: no non-hook wait state; "stop vs wait" is entirely delegated to user Stop hooks;
   no owner documents the wake-on-any-input contract to the agent.

## 5. Minimal-change proposal outline (owners + invariants, no patch)

- **Owner `session/acp_session_impl/cancel.rs` + `goal.rs` (resume)**: introduce an explicit
  re-admission on `resume_goal` — clear the wake barrier (reuse the human-intake clearing already
  at `run_loop.rs:760-770`) and trigger `maybe_start_running_task` so parked
  `SubagentCompleted`/`TaskCompleted` rows promote deterministically. Invariant: *a parked wake is
  either delivered or explicitly settled as a receipt within the resume transition* — never left
  to queue-order luck.
- **Owner `run_loop.rs:2198` (WorkflowCompletionTurn)**: replace `continue` under suppression with
  the same park semantics as subagent wakes (queue or retain-unreported obligation) so suppression
  defers rather than deletes. Invariant: *every completion source shares one admission outcome
  set: deliver now / park with re-delivery / settle with receipt*.
- **Owner `session/acp_session_impl/stop_gate.rs` (or a new idle owner)**: define the wait state —
  when background work is outstanding at turn end, emit an explicit model-facing notice
  (reminder/system-reminder) that the session will be woken by any completion or human input, and
  keep the queue-level "any human input clears suppression and promotes" guarantee as the single
  documented contract (one owner, currently implicit across three files).
- **Invariant to preserve**: explicit human stop must still suppress wakes (anti-restart rule in
  `should_auto_wake_subagent`, `spawn.rs:556-563`) — resume, not completion, is the only
  re-admission trigger.

## Gaps / unknowns

- Whether the pager/`goal resume` UI path enters via `SessionCommand::Prompt` (human intent) in
  every variant is asserted from `run_loop.rs:760` code shape, not traced through the pager RPC —
  medium confidence for the "gate transitively cleared on resume" claim.
- Monitor notification parking (`pending_notifications`) vs drop under suppression was not fully
  traced; `cancel.rs:547-553` clears them on kill-all.
- Evidence docs `notes/delivery-review-shell.md` / `architect-restart-continuation.md` describe the
  checkpoint transaction only; neither addresses pause/resume wake admission — confirming this is
  genuinely open, not merely unmerged.
