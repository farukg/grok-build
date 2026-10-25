# Wave B — lane brief 7: carried-over remainder

Author contract (dispatch AFTER the completion lane lands — shared files goal.rs /
stop-gate surfaces). Maps: completion-delivery-map (§goal) + handoff notes under
docs/fork/handoff/2026-10-24/notes/. Worktree:
/home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover.

## Items (from the owner handoff + goal plan)

1. Dynamic parent-goal evaluation at the actual consumer + real AppView replay
   check: the parent goal must be evaluated against the LIVE consumer state (not a
   stale snapshot) when deciding whether child results satisfy it; the AppView
   replay path gets a real check (assert the replayed view equals the live view for
   the same session state) — see handoff notes (delivery-review-shell.md,
   goal-plan.md checklist) for the recorded gap.
2. Global task/todo display preferences across sessions and restarts: the display
   preferences (F7) persist globally (leader-level), apply to new sessions, and
   survive restarts — coordinate with the request-visibility lane's
   resolve_kind_mode helper: the prefs are INPUT to it, not a second mechanism.
3. Work items durable in F7: task/todo work items live durably (session-restart
   safe) and are reachable in the F7 view; optionally also top-level, queued right
   above the chat input (handoff wording) — implement the durable F7 part; the
   top-level placement only if it does not grow the change materially.
4. Mid-run steer and tool/completion wire contracts: mid-run steer (steering
   messages into a running turn) per the handoff notes; for the wire-contract part
   FIRST assess whether any change touches the gateway wire — if yes: STOP, write
   the evidence-backed wire request (wire/request/receipt + producer/consumer
   change + rollout boundary) into the report for Lead coordination with Sokrates;
   never edit wire formats unilaterally.

## Constraints

Author-only (no cargo/git/build/subprocesses); real-entry-point tests in sibling
*_tests.rs; enums with payloads; no unwrap/expect/panic outside tests; exhaustive
matches; minimal diffs. Report to docs/fork/handoff/2026-10-24/wave-b/lane-report-remainder.md.
Reply ≤10 lines. SUBAGENTS_ALLOWED: no; work silent; never stop silently.
