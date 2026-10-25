# Wave B — lane brief 2: compaction submission + running-agent semantics

Author contract for the compaction lane (dispatched by Lead Krillin after Wave A CI
green; may run as a separate author in parallel with lane 1 — disjoint files).
Defect map: docs/fork/handoff/2026-10-24/wave-b/compaction-map.md (read first).

## Symptoms (Faruk, live-verified 2026-10-24)

1. `/compact` cannot be submitted while the agent is running, unlike queued user
   messages (double-enter). During a RUNNING compaction, incoming inputs (from
   anywhere) abort the compaction. Abort must be manual and explicit only.
2. With a running subagent, a compaction (like user messages) must simply enqueue
   and be sent ASAP; today the queued row hangs forever.

## Causal chains (from the map)

- `/compact` is client-local only (slash/commands/compact.rs:28 -> pending_prompts),
  drained solely when idle (dispatch/queue.rs:328). Commands have no server-side
  enqueue like prompts (run_loop.rs:744 -> prompt_queue.rs:113).
- Once running, ANY turn cancel — including the send-now path for queued messages
  (cancel.rs:132-176) — unconditionally aborts the compaction at
  session/acp_session_impl/cancel.rs:413 (compaction.cancel.request_cancel());
  manual abort (Esc/Ctrl+C, turn.rs:238) is indistinguishable from incidental input.
- With a running subagent the parent stays TurnRunning, so the local `/compact` row
  never drains; compacting the running child instead blocks on the FlushComplete
  turn barrier (memory.rs:368, run_loop.rs:1492-1500) until the 30s timeout.
- NOTE: the bc85 CI failure `finished_coordinator_child_compacts_persisted_checkpoint_
  without_parent_delivery` was a separate NotTerminal race (subagent/mod.rs:1613-1618),
  NOT this stall — do not conflate.

## Required contract (acceptance)

- `/compact` submitted while a turn runs enqueues exactly like a queued user message
  (same FIFO), and the queue drains to the session as soon as possible.
- A running compaction is immune to incidental input arrival (queued messages,
  wakes, notifications); ONLY an explicit manual abort gesture (Esc/Ctrl+C on the
  compaction or an explicit cancel command) cancels it. The abort path must be a
  distinct, named operation — not the generic turn-cancel.
- A running subagent does not stall the parent's queued compaction; the compaction
  proceeds on the parent's turn boundary (or is explicitly deferred with a visible
  reason), and the child-side compaction no longer hard-blocks on the 30s barrier.

## Constraints

Same as lane 1 (author-only subagents; real entry points; phase-oracle tests; code
rules; one consolidated CI run for the wave; gateway/wire changes -> STOP + report).
Tests must cover: compact submitted mid-turn drains in order; input arrival during
running compaction does not cancel (and the compaction completes); manual abort
cancels exactly once; queued compact with running subagent completes or defers
visibly.
