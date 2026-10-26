# Completion delivery, declared waits and model instructions

Current source map. Shell paths are relative to `crates/codegen/xai-grok-shell/src/`.

## Declared wait contract

`session/acp_session_impl/stop_gate.rs` owns `DeclareWait`, its tool schema and
`TURN_WAIT_CONTRACT`. The complete model instruction is supplied through the tool's
description. `turn.rs` includes that tool in the effective top-level tool set.

The turn-start path no longer appends an identical wait-contract SystemReminder to
the conversation for every prompt. This prevents new copies accumulating in history.
Existing historical reminders remain historical records; no conversation data is
removed by this change. Request-visibility records currently enumerate historical
synthetic content again per turn; that separate display/provenance repair remains
active in the F7/request-display work.

`DeclareWait` selects a nonempty subset of live work owned by this session. It is
called alone and ends the current turn. Selected completion or admitted new input
releases it; unrelated registry changes alone do not. The gate suppresses autonomous
goal continuation while the declaration remains effective. The declaration is a
session-local runtime wait and is cleared at recovery rather than restored as a
permanent barrier.

## Completion identities and recovery

- `session/storage/jsonl/completion.rs` prepares classified terminal artifacts,
  publishes them and deduplicates caller receipts under the completion/cancel lock.
- `session/acp_session_impl/completion_admission.rs` appends and acknowledges
  admitted completion projections and consumes typed completion observations.
- `session/workflow/manager.rs`, `workflow/store.rs` and `session/persistence.rs`
  persist/enroll terminal workflow completion before acknowledgement. The actor
  command path in `run_loop.rs` admits workflow and subagent wakes.
- `session/acp_session_impl/delivery_receiver.rs` recovers queued receiver records
  and skips already queued prompt identities. `notification_drain.rs` owns promotion.

The domain contracts are [ADR-001](../../../decisions/001-subagent-results-during-goals.adr.md)
and [ADR-002](../../../decisions/002-attempt-bound-input-admission.adr.md).
They describe admission/projection receipt guarantees, not exactly-once execution
of arbitrary external tool effects.

## Goal pause and human stop

Goal pause and explicit human stop are separate states. `/goal pause` parks autonomous
goal continuation. A human stop arms completion suppression; already durable completion
facts and their receipt state remain available. Normal user re-engagement and recovery
follow the actor's current admission rules. `run_loop.rs` clears the recovered human-stop
barrier; it must not be described as permanently rearmed after every restart.

The dashboard's load path can recover eligible pending work independently of an active
goal. A passive/display-only attach guarantee is not established by these mechanisms.

## Evidence boundary

The wait-instruction placement change is local, unbuilt source work. Regression
execution and live model/PTY acceptance are open. The broader existing completion
and restore paths have source evidence; their end-to-end runtime guarantees require
separate acceptance. The active implementation/evidence owner is the Grok workspace
`grb-f7-request-display.plan.md`; `docs/fork/STATUS.md` identifies staged and local sources.
