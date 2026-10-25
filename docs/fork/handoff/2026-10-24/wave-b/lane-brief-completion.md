# Wave B — lane brief 1: completion/delivery wake + wait semantics

Author contract for the first implementation lane (dispatched by Lead Krillin after
Wave A CI green). Base: grb/owner-takeover @ 5abbd3c (or the green successor).
Defect map: docs/fork/handoff/2026-10-24/wave-b/completion-delivery-map.md (read first).

## Symptoms (Faruk, live-verified 2026-10-24)

1. Completed subagents/workflows trigger NO caller turn when the parent goal is
   paused or the session was stopped with Ctrl+C; only manual input or goal resume
   continues the session.
2. The goal loop re-wakes the agent on every tick even when the agent is waiting on
   running tasks/subagents/monitors/CI; the agent cannot declare "waiting", so it
   either does makework or drifts until it guesses a blocking tool call. Live probe:
   a running monitor does NOT suppress goal re-wakes — there is no wait declaration.

## Required contract (acceptance)

- StopGesture/`HumanStopped` suppression must NOT park completion wakes unpromoted:
  on resume (goal resume OR any new input), parked subagent wakes deliver and trigger
  a caller turn. Workflow completion wakes must not be dropped outright — one
  admission semantics for all completion origins (subagent, workflow, bash task):
  park + promote on wake-window; drop only per explicit policy.
- `resume_goal` (goal.rs) clears the suppression barrier as part of the resume
  transaction; the barrier does not silently survive restarts.
- Wait declaration: an agent turn can end declaring wait-on {task/subagent/monitor/
  workflow/run ids}; while any are live, the goal loop suppresses tick re-wakes;
  completion of any of them wakes the session; ANY new user input also breaks the
  wait. The stop gate (stop_gate.rs AllowStop path) carries the wait set instead of
  a bare stop.
- The goal evaluator treats "waiting on X" as a valid non-terminal stop reason
  (no nudge).

## Owners from the map (start points)

- Suppression barrier: session/acp_session_impl/cancel.rs:97-112 (gate +
  notifications_suppressed + persisted HumanStopped).
- Wake parking: session/acp_session_impl/notification_drain.rs:216-228.
- Resume without barrier clear: session/acp_session_impl/goal.rs:874-1010.
- Workflow wake drop: session/run_loop.rs:2199-2207 (continue under suppression).
- Stop gate: session/acp_session_impl/stop_gate.rs:246-248 (immediate AllowStop when
  no Stop hooks).
- Bash-task wake fallback + subagent park: see map §1/§3 for the three admission
  semantics.

## Constraints

- Author-only for subagents (no cargo/git/subprocesses); Lead runs sig build.
- Tests through real entry points; phase-oracle style (no weakened expected values);
  sibling *_tests.rs files; enums with payloads; no unwrap/expect/panic outside
  tests; exhaustive matches; minimal diffs; comments only for non-obvious constraints.
- One consolidated CI run for this wave AFTER Lead static review + local sig-build
  check + fmt guard — never iterate CI.
- Gateway/wire surface: none expected; if a wire change becomes necessary, STOP and
  report to Lead (coordination with Sokrates required).
