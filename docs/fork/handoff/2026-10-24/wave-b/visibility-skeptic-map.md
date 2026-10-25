# Defect map: background-thing visibility (gb-zombie-7) + goal skeptic multiplicity (gb-skeptic-8)

Codebase: `/home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover` (current truth, read-only inspection).
All paths below are relative to that worktree root unless absolute.

## 1. Background registry — who owns what

There is **no single leader-side task/monitor/subagent registry** that the view reads. The ctrl-t
tasks pane is a **per-CLI, client-side reconstruction** in the pager (`xai-grok-pager`):

- `crates/codegen/xai-grok-pager/src/app/agent.rs:751` — `AgentSession::bg_tasks: BTreeMap<String, BgTaskState>` (one-shot bg commands + monitors).
- `crates/codegen/xai-grok-pager/src/app/agent.rs:755` — `scheduled_tasks` (`/loop` rows).
- `AgentView::subagent_sessions` — subagent rows; `workflow_runs` — workflow rows.
- `crates/codegen/xai-grok-pager/src/views/tasks_pane.rs:694-722` — `TasksPane::sync(bg_tasks, subagents, scheduled, workflow_runs)` builds the interleaved list; kill button per row (`tasks_pane.rs:603,1200`).
- `crates/codegen/xai-grok-pager/src/app/agent_view/render.rs:992-998` — the render-side sync call.

The authoritative runtime state lives leader-side inside the session process:

- `crates/codegen/xai-grok-tools/src/computer/local/terminal.rs:569` — `processes: HashMap<String, ProcessState>` (the terminal/task backend; owns OS processes, `owner_session_id` per task at `terminal.rs:365`).
- `crates/codegen/xai-grok-tools/src/implementations/grok_build/scheduler/actor.rs:199,850` — scheduler actor re-announces persisted `/loop` tasks (`announce_existing_tasks`) at every actor start; pager gives re-announced rows a synthetic `created_at` (`actor.rs:861` comment).
- Subagent truth: subagent coordinator + per-session-dir records; healed by
  `crates/codegen/xai-grok-shell/src/agent/subagent/mod.rs:2884` (`reconcile_live_orphaned_subagents`), invoked from
  `crates/codegen/xai-grok-shell/src/session/acp_session_impl/notification_drain.rs:470,527,614`.

### How a new CLI attach populates the view (snapshot path)

1. Replay: `session/load` replays the persisted transcript. `TaskBackgrounded` notifications are
   persisted at `crates/codegen/xai-grok-shell/src/tools/notification_bridge.rs:343-348` (persistence channel) and simultaneously fire-and-forgotten to the gateway (`notification_bridge.rs:352-357`).
2. Pager replay handling: `crates/codegen/xai-grok-pager/src/app/acp_handler/background.rs:63-103` (`handle_task_backgrounded`, `restored_from_replay`), tombstones at `background.rs:575-581`; scheduled rows at `background.rs:260-315` (keyed by `task_id`, occupied-entry merge).
3. Live re-announce: scheduler `ScheduledTaskCreated` broadcast on actor start (above); subagent lifecycle via `crates/codegen/xai-grok-pager/src/app/acp_handler/subagent_lifecycle.rs:144+` with a reduce state machine keyed by `child_session_id` + attempt key.
4. Reconnect dedup: `crates/codegen/xai-grok-pager/src/app/agent_view/mod.rs:777-784` — `last_applied_event_seq` / xAI-stream highwater drops duplicate `session/update`s; reconnect sends `_meta.cursor` so only the post-cursor tail replays. **But line 781: "Durable subagent lifecycle events bypass that drop check"** — replay-vs-live double delivery of subagent lifecycle is NOT eventId-deduped.

### The reconciler that is missing in the TUI

The shell has a snapshot mechanism — `EmitBackgroundTasksSnapshot` → `SessionUpdate::BackgroundTasks`
(`crates/codegen/xai-grok-shell/src/session/acp_session_impl/background_tasks.rs:23-76`) — but:
- it is **skipped when gateway-backed** (leader mode!) at `background_tasks.rs:35` and `:53`, and
- the **TUI pager never consumes `SessionUpdate::BackgroundTasks`** (zero non-test references in `xai-grok-pager`; only `headless.rs` handles TaskBackgrounded events).

So in Faruk's leader topology the tasks pane has **no authoritative reconciliation source at all**;
it is only replay + whatever live notifications happen to arrive after attach.

## 2. gb-zombie-7 causal chains

### (a) Running things NOT visible on a new grok invocation — HIGH confidence

Chain:
1. Tasks/monitors are owned by the leader-side session that spawned them: `owner_session_id` on every
   backend entry (`terminal.rs:365`, reparent/kill-by-owner commands `terminal.rs:1149-1176`), and monitor
   events are filtered per owner (`crates/codegen/xai-grok-tools/src/implementations/grok_build/monitor/types.rs:122-133` — "an event surfaces only when its owner matches the draining session").
2. A new grok CLI invocation attaches as (or creates) a session view whose `bg_tasks` map starts empty.
3. Population depends on (i) replay of the OLD session's transcript — only if that exact session is
   loaded — and (ii) the skipped-in-leader-mode `BackgroundTasks` snapshot (`background_tasks.rs:35,53`).
4. Result: still-running tasks in the leader (kept alive up to `BACKGROUND_MAX_RUNTIME`,
   `terminal.rs:1566`) are invisible in the fresh CLI. Nothing lists them cross-session; the
   `ExternalRoster` (`crates/codegen/xai-grok-shell/src/leader/roster_merge.rs:30-46`) only merges
   session-list rows from the cursor-worker door, not task rows.

Defect site: missing authoritative task-listing surface per CLI attach — the emit path exists
(`SessionUpdate::BackgroundTasks` / `list_tasks_metadata` `terminal.rs:2721`) but is disabled exactly
in gateway/leader mode and unconsumed by the TUI.

### (b) MANY DUPLICATES of the same entries — MEDIUM confidence (two candidate mechanisms, both real in source)

1. **Replay/live double-apply for subagent rows**: `agent_view/mod.rs:781` explicitly exempts durable
   subagent lifecycle events from the eventId dedup drop. If a lifecycle event is delivered both by
   replay and by a live re-emit (the notification bridge stamps a NEW `eventId` per emission —
   `notification_bridge.rs:335-338` `stamp_event_id`), the reduce path (`subagent_lifecycle.rs:199-260`)
   accepts it; a *new attempt key* (`ApplyNewAttempt`) creates a fresh row/attempt in the strip even for
   the same logical child. Scheduler re-announces (`actor.rs:199`) likewise re-deliver
   `ScheduledTaskCreated` — deduped only when the `task_id` matches exactly.
2. **Per-restart re-arm with new ids**: `/loop` re-arming after a leader restart re-spawns iterations
   with new `subagent_id`s (chain-anchor reset at `actor.rs:820-844`), each appearing as a new row;
   monitors backgrounded across restarts get new `task_id`s per tool call. All rows are kill-button
   "dismissible" (`tasks_pane.rs:1200`) even when the underlying entry no longer exists — kill just
   removes the local map row (`crates/codegen/xai-grok-pager/src/app/dispatch/turn.rs:866`).

### (c) Can real zombies arise? — YES, two windows

1. **OS processes outliving the registry**: every spawn uses `kill_on_drop(true)`
   (`crates/codegen/xai-grok-tools/src/sandbox_launch.rs:13`, `computer/local/lifecycle.rs:181`, etc.).
   `kill_on_drop` only fires on `Drop` **inside a live process**. A SIGKILL'd (or crashed) leader never
   drops its `Child` handles → process groups are orphaned and keep running; the transcript then shows
   them as running forever (`restored_from_replay` tombstones only appear when a `TaskCompleted` was
   persisted — `background.rs:472-581`).
2. **Stale "running" registrations**: subagent records saying running-but-dead are healed only when the
   parent session drains (`notification_drain.rs:527,614`) — with the CLI detached and the session idle,
   nothing triggers reconciliation, so ghost rows persist in any view that replays them.

## 3. gb-skeptic-8 — intended vs bug: **INTENDED N=3 PANEL, not a spawn bug**

- `crates/codegen/xai-grok-shell/src/session/goal_classifier.rs:3-6` — "it spawns N independent skeptic
  subagents in parallel", description constant `"goal achievement skeptic"` at `:71` (stable label, so
  all rows show the SAME title), `GOAL_VERIFIER_SKEPTIC_COUNT: u32 = 3` at `:78`, clamp 1..=5 at `:82-83`.
- Same prompt template (`GOAL_VERIFIER_PROMPT_TEMPLATE`, `goal_classifier.rs:73`), per-index scratch dirs
  (`goal_tracker.rs:351-354`) so N skeptics never collide — i.e. identical-looking spawns are deliberate.
- Trigger: `update_goal(completed: true)` → `verify_goal_candidate`
  (`crates/codegen/xai-grok-shell/src/session/acp_session_impl/goal.rs:164`) — one panel per completion
  claim, not per model round. Re-evaluation rounds are re-triggered because the rejection directive pushes
  the model to re-claim (`goal_support.rs:474-515`), up to `GOAL_CLASSIFIER_MAX_RUNS_DEFAULT = 10`
  (`goal_classifier.rs:29`).
- Single-flight guards EXIST: `verifying_in_flight` latch + attempt-slot reservation
  (`goal.rs:194-203,229-237`) and duplicate-claim rejection (`goal_support.rs:125-127`
  `ConcurrentInFlight`; `SubagentSpawnOutcome::Duplicate` at `goal_support.rs:705-723`). No missing dedup
  owner was found — the dedup owner is `goal_classifier.rs` fan-out + `goal.rs` attempt reservation.

Desired "exactly ONE skeptic": already supported as configuration — `GROK_GOAL_VERIFIER_N=1` or remote
`goal_verifier_count = 1` (clamped `GOAL_VERIFIER_SKEPTIC_MIN=1`, `goal_classifier.rs:82`). Note
`goal_classifier.rs:826`: in fan-out panels skeptic-0's not-refuted vote alone does not count; at N=1 the
panel semantics collapse to that single skeptic's verdict, which matches the desired contract.

## 4. Gaps

- The exact duplicate-row mechanism Faruk sees was not reproduced end-to-end from source; both candidates
  in §2(b) are supported by code but which dominates depends on runtime notification routing (needs a
  leader+CLI repro with event logging).
- `xai-computer-hub` (task hub) paths were not audited; if tasks can be hosted in a hub process rather
  than the session process, the zombie window in §2(c)(1) shifts accordingly.
- Workflow-run row lifecycle (`workflow_runs`) only skimmed.

## 5. Minimal-change proposals (owner + invariants, no patch)

1. **Attach reconciliation (kills both invisibility and ghosts)** — owner: `xai-grok-pager` tasks pane +
   `xai-grok-shell` snapshot emit path. Invariant: after `session/load` completes, `bg_tasks` equals the
   leader's `list_tasks_metadata` truth (replace-merge by `task_id`, clear missing). Re-enable the
   `BackgroundTasks` snapshot in gateway-backed mode (`background_tasks.rs:35,53`) and add a TUI consumer.
2. **Cross-session visibility** — owner: leader roster (`leader/roster_merge.rs`) or the session-view
   routing. Invariant: still-running leader tasks of ANY session are listable from any attached CLI
   (tagged with owning session), removing the `owner_session_id` blind spot (`monitor/types.rs:122`,
   `terminal.rs` owner filters).
3. **Orphan-proof leader shutdown** — owner: leader process supervision. Invariant: leader exit tears
   down spawned task process groups (process-group kill on shutdown / watchdog), so `kill_on_drop`
   gaps (`sandbox_launch.rs:13`) cannot orphan them.
4. **Skeptic count** — owner: `goal_classifier.rs:78-83` (already configurable). Set `goal_verifier_count=1`
   in remote config for the "exactly one skeptic accepts/rejects" contract; no code change required.
