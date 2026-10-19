# M11 — agent and human control over subagents

Branch: `grb/m11` (based on `main` `b540b07e`; merge `main` before continuing).

## Goal
The calling agent and the human can see and control subagents: queue or steer messages,
stop / pause / resume, and always see a typed terminal state with the interruption cause and who
caused it (agent or human). Plan and file:line evidence: `docs/fork/recon/m11-subagent-control.md`
(B1 typed terminal state, B2 messaging, B3 stop/pause/resume, B4 visibility).

## State (2026-09-30)
- On `main`: typed state types (`common/xai-tool-types/src/subagent_state.rs`: `SubagentActor`, `InterruptionCause`,
  `SubagentState`, `model_text()`), and B1 wiring: `SubagentResult.state` replaces `success/error/cancelled`
  (`success()`, `error()`, `is_interrupted()` are derived). `SubagentCancelTarget::SubagentId` carries actor and
  `SubagentCancelDisposition::{Stop, Pause}`; every cancel site (turn, session stop, teardown, workflow, queue) records its
  `InterruptionCause` in `PendingDisposition::Interrupted(cause)` (first cause wins, a stop overrides a pause), and
  `finish_child` stamps it on the result. `explicitly_killed` is derived from the disposition. Only a pause leaves the
  completed child `wake_eligible`. `SubagentSnapshotStatus::Cancelled { cause }`. `x.ai/subagent/cancel` takes
  `mode: stop|pause` (default stop); `SubagentSnapshotDto` carries `interruption` (`cancelReason` keeps the wording).
- The old uncompiled draft is archived as `archive/m11-b1-draft`; `archive/m11-full-wip` is the older experiment.

## Next steps
1. B3 model tool `control_subagent { subagent_id, action: pause|stop }` (gated), `kill_task` already goes through the
   same backend call as `ExplicitStop { ParentModel }`. Resume of a paused child = `send_subagent_message` (wakes) or
   `task(resume_from)`, which now wakes a finished child under its own id.
2. B4 visibility: `list_subagents` on a coordinator `ListOwned` query; `outcome_words` in `T/../reminders/task_completion.rs`
   still says "was cancelled" for every cause (use `InterruptionCause::model_text`); `SessionUpdate::SubagentFinished`
   should carry `interruption`; persisted `SubagentMeta.status` is still a string (old metas must keep loading).
3. B2 human `delivery` field on `SendSubagentMessageRequest` through `resolve_delivery`.
4. Pager: parse `interruption` in task rows and the child header, `Action::PauseSubagent` next to `KillSubagent`.
