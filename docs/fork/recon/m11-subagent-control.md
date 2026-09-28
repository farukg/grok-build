# M11 recon — caller control + visibility of subagents

Base: `main` b695fdc9 (grb-m4 / grb-m2 read only). Paths are relative to `crates/codegen/` unless they start with `common/`.
Short names: T=`xai-grok-tools/src/implementations/grok_build`, S=`xai-grok-shell/src`, P=`xai-grok-pager/src`.

## Verdict on the triggering incident
The architect's claim is **true under Faruk's config**, but the feature does exist. Mid-run messaging is already built:
`send_subagent_message` supports steer|queue|interject. It sits behind `Feature::ActiveAgentMessages`
(registry.rs:281-287, `default_enabled: false`, env `GROK_ACTIVE_AGENT_MESSAGES`). `~/.grok/config.toml [features]` does not set it,
the env var is unset, and the generator template (`grok-config-toml.template` `[features]`, line 43ff) lacks it as well.
Because the flag is off, `xai-grok-agent/src/builder.rs:824-835` strips the tool from every toolset. The same flag also gates the
human ext method (S/extensions/subagent_message.rs:102, S/agent/mvp_agent/subagent_spawn.rs:82-88).
`cancel_subagents_on_turn_cancel = "always_continue"` is set in both config and template, so a parent Esc does not cancel children
(P/app/dispatch/turn.rs:24).

## (a) Capability matrix

| Capability | Model/agent | Human TUI | Evidence |
|---|---|---|---|
| Send to running child: steer | exists-behind-flag (default) | wire exists-behind-flag; **no TUI caller** | T/send_subagent_message.rs:17-24,36-45,55; ext S/extensions/subagent_message.rs:1,111; the only pager hit is the tool-result renderer P/acp/subagent_message.rs:85 |
| Send: queue (after current turn) | exists-behind-flag (`delivery:"queue"`) | wire: legacy `queue: bool` only | subagent_message.rs:17-22 has no `delivery`, only `resolve_delivery(None, req.queue)` :111 |
| Send: interject | exists-behind-flag | missing (wire cannot express it) | same |
| Message to a *finished* child → wakes same identity | exists-behind-flag | wire only | T/task/coordinator/wake.rs:54-146 (`run_in_background=true`, new cancel token) |
| Child → parent message | exists-behind-flag (granted child `AgentMessageSender`) | n/a | T/send_subagent_message.rs:264-281; T/task/agent_message_sender.rs:59-91 |
| Stop / cancel a child | exists: `kill_task` falls back to subagent cancel, outcome string `"killed"` | exists: `Action::KillSubagent` → `x.ai/subagent/cancel` | T/kill_task/mod.rs:217-252; P/app/actions.rs:412,1592-1597; P/app/agent_view/panes.rs:774,1049; S/extensions/task.rs:426-436 |
| Pause | **missing** (no concept anywhere; `rg -i pause` in task/ and subagent/ finds only a test-clock comment) | **missing** | T/task/coordinator/queue.rs:63 |
| Resume same identity after stop | **missing**: cancel sets `wake_eligible=false`, so later sends return `NotActiveOrFinalizing` | missing | T/task/coordinator.rs:1252; T/task/coordinator/agent_targets.rs:93-97,205-214 |
| Resume as a new child from a stopped one (`resume_from`) | exists: in-memory lookup ignores the cancel; durable meta accepts `cancelled` | via M4 `x.ai/subagent/resume` (in progress) | T/task/coordinator.rs:854-868; S/agent/subagent/mod.rs:1397-1398 |
| Resume of a *running* child | exists as a rejection "still running" on main; M4 changes it to queued delivery | M4 | S/agent/subagent/handle_request.rs:448-453; grb-m4 `task/resume.rs:180-212` |
| Status per id | exists: `get_task_output` snapshot (Initializing/Running/Completed/Failed/Cancelled) | wire `x.ai/subagent/get` (String status); pager does not call it and builds rows from notifications instead | T/task_output/mod.rs:713-750,878; S/extensions/task.rs:437-467 |
| List all own children | **missing** for the model (`ListActive` feeds only compaction and stop_gate) | wire `x.ai/subagent/list_running` (running only, no caller in the pager); pager tasks pane from notifications | S/session/compaction.rs:1432; S/session/acp_session_impl/stop_gate.rs:148; T/task/coordinator/query.rs:152-166 |
| Completion/idle notice | exists; says only "was cancelled" | scrollback row "cancelled" | T/../../reminders/task_completion.rs:489-498 |
| Queued (at concurrency cap) as its own state | missing: shown as `Initializing` | missing | T/task/coordinator_state.rs:~955-966 (`queued_snapshot`) |
| **Why/by whom interrupted** | **missing (free text at best)** | **missing** | see the next section |

### Interruption cause: where it is lost
- The cause is typed only when the request comes in: `SubagentCancelTarget::{SubagentId, ParentPromptId, ParentSession, WorkflowRunId}` (T/task/types.rs:652-659).
  The model and the human both use `SubagentId` (T/task/backend.rs:624-635; S/agent/mvp_agent/agent_ops.rs:2835-2846), so an explicit stop cannot tell agent from human.
- The coordinator erases the cause. `cancel_one`, `cancel_parent_prompt`, `cancel_parent_session` and `teardown_session_children` all set
  `PendingDisposition::Cancelled`, a two-variant enum `Live | Cancelled` (T/task/coordinator_state.rs:477-484), plus the bool `explicitly_killed`
  (T/task/coordinator.rs:1320-1378, 1434-1465, 1534-1565).
- The result is `SubagentResult { success: bool, cancelled: bool, error: Option<String> }`, and `status()` returns `&'static str` (T/task/types.rs:440,500-534).
  The runner hard-codes `"Subagent was cancelled"` (S/agent/subagent/attempt_runner.rs:159-169). That is a §MOD.1f flat status + separate error.
- The snapshot has `Cancelled { reason: Option<String> }` (types.rs:636), and `terminal_snapshot` copies `result.error` into it (coordinator_state.rs:990-994).
- Restart is text only: `ORPHAN_RECONCILE_REASON = "interrupted by process restart"` and `LIVE_ORPHAN_RECONCILE_REASON` (S/agent/subagent/mod.rs:2334-2335).
  The meta is flipped to `status:"cancelled"` with that error (mod.rs:2358-2393).
- Wire and persistence are strings. `SubagentMeta.status: String` (mod.rs:2047-2048).
  `SessionUpdate::SubagentFinished { status: String, error: Option<String> }` (S/extensions/notification.rs:799-828).
  `SubagentSnapshotDto.status: String`, `cancel_reason` (S/extensions/task.rs:221-343). `SubagentCancelOutcome::AlreadyFinished { status: String }` (types.rs:672-676).
- The pager is strings too. `SubagentAttemptInfo.status: Option<Arc<str>>` (P/app/subagent.rs:61-62). The takeover header branches on `== Some("completed")`
  (P/app/agent_view/subagent_takeover.rs:304-322). The lifecycle reducer knows only `Running | Finished` (P/app/subagent/lifecycle.rs:25-28).
- What reaches the model: `get_task_output` shows the reason text or "Subagent was cancelled" (T/task_output/mod.rs:745-750). The reminder shows "was cancelled" and nothing else.

## (b) Minimal reuse-based design

**B1 — Typed terminal state (the root fix).** One boundary type in `common/xai-tool-types` (it already hosts `SubagentResumeFallback` in M4):
```
enum SubagentInterruption { ExplicitStop{by: Principal}, Paused{by: Principal}, ParentTurnCancelled{prompt_id},
  ParentSessionStopped, ParentTeardown, WorkflowCancelled{run_id}, ProcessRestart, OrphanedLiveParent }
enum Principal { Agent{session_id}, Human }
enum SubagentTerminal { Completed{..}, Failed{kind: FailureKind /*M4*/, message}, Interrupted(SubagentInterruption) }
```
- Replace `PendingDisposition::Cancelled` with `Cancelled(SubagentInterruption)`. Each cancel site maps its `SubagentCancelTarget` 1:1.
  `SubagentId` gains a `Principal`, carried by `SubagentCancelRequest` from the kill_task, ext and pause entry points.
- `explicitly_killed` becomes a derived projection of the cause (§MOD.6).
- `SubagentResult` replaces `success/cancelled/error` with `SubagentTerminal` (keep `backgrounded` as a separate handle signal).
  `status()` becomes an `IsVariant`/strum projection.
- `SubagentSnapshotStatus::{Failed, Cancelled}` collapses into `Terminal(SubagentTerminal)`, or gets `Cancelled{interruption}`.
  Split `Initializing` from `Queued{position}`.
- Wire boundary, additive for old clients:
  - keep `status` as the strum string;
  - add `interruption` (serde tagged) to `SubagentFinished`, `SubagentSnapshotDto` and `SubagentMeta`;
  - old meta without it parses to a named `LegacyUnknown` variant at the one parse site. No silent default.
- The orphan reconcile writes `ProcessRestart` or `OrphanedLiveParent` instead of the reason consts.

**B2 — Messaging (queue|steer).**
- Model: nothing new in code. Enable the flag (M6 template). Keep `delivery` as it is (T/send_subagent_message.rs:55).
- Human: add `delivery: Option<SendSubagentMessageDelivery>` to `SendSubagentMessageRequest` and route it through the existing
  `resolve_delivery(req.delivery, req.queue)` (already the SSOT). Interject then works for humans too.
- Pager: the takeover composer (M6) sends with Enter=steer and a modifier for queue.

**B3 — Stop / pause / resume, reusing `cancel_one` and the wake path.**
- `pause` is `cancel_one` with `Paused{by}`. It sets `wake_eligible = matches!(cause, Paused{..})` at coordinator.rs:1252.
- Resume of a paused child uses the existing wake (wake.rs:54ff), so identity, address and worktree are kept. The wake prompt is prefixed with a typed "resumed after pause" notice.
- `stop` keeps today's semantics: terminal, not wakeable, `resume_from` still possible as a new child.
- Model surface:
  - one gated tool `control_subagent { subagent_id, action: pause|stop }`, both calling `backend.cancel(id, cause)`;
  - `kill_task` keeps delegating to the same backend call as `ExplicitStop{Agent}` (SSI, no second cancel path);
  - resume = `send_subagent_message` to a paused id (already wakes), or M4's resume route.
- Human: `x.ai/subagent/cancel` gains `mode: stop|pause` (serde default stop). The pager gets `Action::PauseSubagent` next to `KillSubagent` (panes.rs:774/1049), and resume goes through the composer or M4's resume.

**B4 — Visibility.**
- Model status tool: add a coordinator `ListOwned{parent_session_id}` next to `handle_list_running` (query.rs:152). It returns
  `SubagentInspection` for active, pending, queued and retained-completed children.
- Expose it as the model tool `list_subagents` (same flag). Reuse `SubagentSnapshotDto` (S/extensions/task.rs:221) as the single projection,
  and move it into the tools crate so the ext method and the tool share it.
- Extend `x.ai/subagent/list_running`, or add `x.ai/subagent/list`, on the same query.
- `get_task_output` (task_output/mod.rs:745) and `outcome_words` (task_completion.rs:494) render the interruption (e.g.
  "was paused by you — send a message to resume", "cancelled because the parent turn was cancelled"). A Paused notice should not auto-wake the parent.
- Pager:
  - replace `status: Option<Arc<str>>` with a parsed `SubagentTerminal`;
  - `SubagentLifecyclePhase` gains `Paused`;
  - the tasks-pane rows and the takeover header (subagent_takeover.rs:296-322) match on it and show the cause and principal.

## (c) Sequencing
**Can start now (no conflicting files):**
- B2 human `delivery` field in S/extensions/subagent_message.rs. M4 does not touch it: M4's diff touches extensions/mod.rs, notification.rs and task.rs.
- Defining the B1 enums in a *new* `common/xai-tool-types` file.
- The model tool descriptions/schemas for `control_subagent` / `list_subagents` (new files).

**Must wait for M4** (M4 edits task/types.rs, coordinator.rs, coordinator_state.rs, backend.rs, task/mod.rs, notification.rs, extensions/task.rs, subagent/mod.rs, handle_request.rs, spawn.rs):
- all of B1 wiring;
- B3 cause tagging and wake eligibility;
- B4 `ListOwned` and the DTOs;
- `FailureKind` must come from M4's typed failure reasons;
- the resume UX must build on M4's `route_subagent_resume` (grb-m4 task/resume.rs:180-212, currently Queue-only for running targets).

**Must wait for M6:**
- flag enable in the template (without it M11 is invisible to Faruk);
- takeover composer (B2 TUI);
- any takeover-header work also rebases on grb-m2 (subagent_takeover.rs, 220 changed lines).

## (d) Risks
- **Wire compatibility.** `status` strings are consumed by old pagers, headless (P/headless.rs:1659), workflow_ingest (`"failed"|"interrupted"`), and meta on disk. Keep strings as a strum projection and make the new fields additive.
- **Pause is lossy.** A cancel token aborts the in-flight tool call and turn (attempt_runner.rs:159). Pausing mid-edit can leave a partial worktree state. The resume notice must say so. A true "pause at next safe point" would need a Steer-like cooperative stop, which is not modeled today.
- **Wake eligibility.** Flipping `wake_eligible` for Paused must not reopen Stop, turn-cancel or teardown. It must also respect `spawn_blocked_sessions` (wake.rs:90-99) after a user Stop.
- **Races.** A cancel racing a natural completion should resolve completed-wins, as today. The first cause wins when several cancels arrive (e.g. ParentSession after ExplicitStop).
- **Semantic overlap with M4.** M4 makes `resume_from` on a running child enqueue a message (Queue). M11 must keep one resume entry point: resume of a Paused child wakes; resume of a stopped child spawns `resume_from`.
- **Flag scope.** Enabling `active_agent_messages` also grants children the upward/peer messaging capability (builder.rs:820-829; agent_rebuild.rs:438-441). Quotas exist (agent_message_sender.rs:107-125) but this is a behavior change for every child.
- **Tool-description conflict.** The `send_subagent_message` description forbids using it to tell a child to stop (send_subagent_message.rs:212). That is correct once `control_subagent` exists, but the two descriptions must point to each other.
- **§MOD debt touched.** `SubagentResult` flat bools + error, `SubagentMeta.status: String`, and the pager's string status are pre-existing §MOD.1f/1b violations on this path. B1 removes them. Partial adoption (adding a cause beside the bools) would be a new violation.
