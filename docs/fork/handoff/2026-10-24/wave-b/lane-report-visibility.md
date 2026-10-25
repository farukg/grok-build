# Wave B — lane report 6: background-thing visibility + zombie prevention

Status: implemented (author-only; not compiled — no cargo/build/git per lane contract).
Worktree: `/home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover`.

## What was implemented

### 1. Authoritative `BackgroundTasks` reconciler reaches the TUI

- **Finding first (map correction):** the map's claim that the snapshot emit is
  "skipped in gateway/leader mode at `background_tasks.rs:35,53`" is structurally
  true upstream but **unreachable in this fork**: every flip-off site gates on
  `session_computer_sessions` being non-empty
  (`agent/mvp_agent/session_setup.rs:726,1194,1445`, `agent/mvp_agent/agent_ops.rs:1155-1157`),
  and `resolve_session_computer_sessions` returns `Ok(None)` unconditionally
  (`agent/mvp_agent/mod.rs:212-218`). So `emit_local_background_tasks` is always
  `true` here; the shell persists + gateway-forwards
  `SessionUpdate::BackgroundTasks` on task background/complete
  (`tools/notification_bridge.rs:98-113`, `session/acp_session_impl/updates.rs:1047-1095`)
  and on `session/load` (`session_setup.rs:1456 emit_background_tasks_snapshot_and_wait`).
  The flag stays untouched: it is live protection for the (fork-disabled)
  session-computer remote rail, not dead code to delete.
- **The real gap was the consumer:** zero non-test pager references to
  `SessionUpdate::BackgroundTasks`. Added:
  - `xai-grok-pager/src/app/acp_handler/background.rs` —
    `apply_background_tasks_snapshot(agent, tasks, truncated, is_replay)`:
    replace-merge `AgentSession::bg_tasks` by `task_id`; a complete
    (`!truncated`) snapshot removes rows the leader registry no longer holds
    (ghosts) and their `bg_tool_call_to_task` routing entries; a truncated
    snapshot never clears (not the complete set). Terminal transitions update
    status/exit/signal/end timestamps and stop the "Task started" bullet
    animation, but push **no** new scrollback blocks (incremental
    `x.ai/task_completed` owns terminal blocks, live and replayed).
  - `session_notification.rs` — new `XaiSessionUpdate::BackgroundTasks` arm in
    the big match, so the snapshot rides the same highwater dedup + reconnect
    cursor advancement as other applied updates.
  - `xai-grok-shell/src/extensions/mod.rs` — `background_task` module is now
    `pub` so the first-party pager can name the wire row type.
- Invariant delivered: after `session/load` completes and the first snapshot
  applies, the tasks pane's `bg_tasks` equals the leader's
  `list_tasks_metadata` truth for this session (rows are already
  owner-session-filtered shell-side, `extensions/background_task.rs:110-120`).

### 2. Exact-`eventId` dedup for all background-thing lifecycle events, replay + live

The highwater (`last_applied_xai_event_seq`) cannot cover lifecycle events:
they legitimately arrive out of stream order (direct-to-gateway vs FIFO), which
is why subagent lifecycle bypassed the drop check. The correct key for
replay/live double-apply is the **raw eventId**: a persisted event re-delivered
carries its original id; a genuinely new emission gets a fresh one.

- `xai-grok-pager/src/app/agent_view/mod.rs` —
  `AgentView::applied_background_event_ids: VecDeque<String>` (bounded,
  `APPLIED_BACKGROUND_EVENT_ID_LIMIT = 512`) + `SessionReload` captures it.
- `agent_view/session.rs` — `admit_background_lifecycle_event(Option<&str>)`
  (None = synthetic/internal payload, always admits); `bind_session_id` clears;
  `begin_session_reload` captures-and-clears (the replay rebuild must re-apply
  onto the fresh transcript); the reload-failure path restores the pre-window
  memory (same treatment as the highwaters).
- Admission points (all real wire entries):
  `handle_task_backgrounded`, `handle_task_completed`, `handle_monitor_event`,
  `handle_scheduled_task_created`, `handle_scheduled_task_fired`,
  `handle_scheduled_task_deleted` (covers both the dedicated ext methods and
  the `x.ai/session_notification` / replay `x.ai/session/update` carriers,
  which route into the same handlers), the subagent lifecycle path in
  `handle_session_notification_with_origin` (before the reducer — a re-delivered
  spawn/finish must not render a second strip row or attempt), and the
  `BackgroundTasks` snapshot arm.
- `subagent_lifecycle.rs` — the deferred `AwaitSpawn` finish copy strips its
  `eventId` before storage, so the internal re-dispatch when its spawn arrives
  is not caught by the dedup (only wire deliveries are).

### 3. Re-arm reconciliation keyed by stable identity

- Pager `handle_scheduled_task_created`: the provisional `/loop` row purge is
  now keyed by loop identity (prompt + human_schedule) instead of a blanket
  `provisional-*` purge — a sibling `/loop`'s instant-feedback row survives
  until its own confirmation. Re-announces merge by the stable persisted
  `task_id` (occupied-entry merge preserves `created_at`, so restart
  re-announces neither duplicate nor reorder rows).
- `scheduler/actor.rs` itself needed no change: re-announce
  (`announce_existing_tasks`, actor.rs:850-881) already keys by the persisted
  task id and pager merges occupied entries; per-fire `subagent_id`s are the
  `/loop` iteration design (map §3), and fresh `SchedulerClock` generations ride
  only diagnostic meta the pager ignores. The occurrence-journal layer
  (`occurrence_journal.rs`, all `dead_code`-marked "wired by durable one-shot
  actor layer") already keys by stable occurrence ids; wiring it into the actor
  is that layer's own follow-up.

### 4. Leader-startup orphan sweep — pure decision logic

- New `xai-grok-shell/src/session/acp_session_impl/background_orphan_sweep.rs`
  (+`_tests.rs`), declared and re-exported in `session/acp_session.rs`
  (`RegisteredTaskProcessGroup`, `SweepFacts`, `SweepDecision`,
  `StartupSweepPlan`, `classify_registered_group`, `decide_startup_sweep`).
- Model: a persisted per-spawn process-group registry row stamped with the
  spawning **leader generation**; startup facts (current generation,
  verified-live leader generations, confirmed-alive pids) are gathered by the
  caller — the module sends/probes/simulates no signals. Decisions (enum with
  payloads, exhaustive): `OwnedByLiveLeader{generation}` (current or other live
  generation — never touched), `ReapOrphanedGroup` (spawning generation dead,
  pid alive — the SIGKILL-orphan window `kill_on_drop` cannot close),
  `AlreadyGone` (pid dead — stale registry row, nothing to signal),
  `UnknownOwner` (pre-generation-stamping row — never kill what cannot be
  attributed). The plan dedupes process groups to reap and keeps a per-task
  decision log. Wiring (pidfile writes at spawn in `computer/local/terminal.rs`,
  liveness probing, actual kills at leader startup in `leader/server.rs`) is
  owned by those files and deliberately out of this lane's author-only scope.

## Files changed

Shell (`xai-grok-shell`):
- `src/extensions/mod.rs` (module visibility)
- `src/session/acp_session.rs` (module declaration + re-export)
- `src/session/acp_session_impl/background_orphan_sweep.rs` (new)
- `src/session/acp_session_impl/background_orphan_sweep_tests.rs` (new)

Pager (`xai-grok-pager`):
- `src/app/agent_view/mod.rs` (dedup field, reload snapshot field, cap const)
- `src/app/agent_view/session.rs` (admission method; bind/reset; reload take/restore)
- `src/app/acp_handler/mod.rs` (import)
- `src/app/acp_handler/background.rs` (snapshot consumer; 6 admission checks; identity-keyed provisional replacement)
- `src/app/acp_handler/session_notification.rs` (lifecycle admission; `BackgroundTasks` arm)
- `src/app/acp_handler/subagent_lifecycle.rs` (strip id on deferred internal copy)
- `src/app/acp_handler/tests/mod.rs` (helpers: `ext_with_event_id`, `task_backgrounded_update`)
- `src/app/acp_handler/tests/background_tasks.rs` (8 new tests + wire-row helpers)
- `src/app/acp_handler/tests/scheduled_tasks.rs` (3 new tests + created-notif helper)
- `src/app/acp_handler/tests/subagent_attempt_lifecycle.rs` (1 new test + block-count helper)

Untouched per lane contract: `cancel.rs`, `goal.rs`, `notification_drain.rs`,
`stop_gate.rs`, `run_loop.rs`, `terminal.rs`, `leader/*`, scheduler wire types.

## Tests (all through real entry points: `handle_ext_notification` / direct handler calls)

- Reconciler visibility: unseen running row inserted from a live snapshot
  (monitor kind, replay flag false); complete snapshot clears ghost rows and
  their stdout-routing entries; truncated snapshot keeps missing rows;
  snapshot-discovered terminal state finishes the started block's animation
  without pushing a completion block.
- Dedup across replay+live: `TaskBackgrounded` live (dedicated method) then
  replay (`x.ai/session/update`) with the same eventId renders one block;
  duplicate `TaskCompleted` pushes exactly one completion block; duplicate
  snapshot id drops (extra row in the dup payload must not appear), fresh id
  applies; subagent spawn live-then-replay same id renders one strip row and a
  fresh-id finish still applies; reload window re-admits replayed ids and a
  failed reload restores the pre-window memory; rebinding clears it.
- Identity stability across restart snapshots: scheduler re-announce (fresh
  event id, same stable task id) merges into one row preserving `created_at`;
  provisional replacement is identity-keyed (sibling survives); exact-id
  re-delivered creation cannot overwrite newer row content.
- Sweep ownership decisions (pure): current-generation keep, other-live-generation
  keep, dead-generation+alive-pid reap, dead-pid already-gone, untagged
  unknown-owner; plan dedupes shared process groups and keeps the decision log;
  empty facts never reap.

## Open questions / risks

1. **Not compiled** (author-only lane): formatting drift vs rustfmt and any
   cross-crate visibility nit in the new `pub mod background_task` export are
   unverified until the owning builder runs `sig build`.
2. Provisional `/loop` rows whose confirmed create carries an edited prompt
   (model changed it between preview and `scheduler_create`) would linger as
   dismissible rows; matching on prompt+schedule is the strongest identity the
   wire offers without a preview↔task correlation id.
3. The orphan sweep's registry writer (pidfile per spawn in `terminal.rs`) and
   its leader-startup invocation are follow-ups for their owners; until wired,
   the decision core is exercised only by its pure tests.
4. Duplicate-row mechanism dominance (map §4 gap) remains runtime-unproven; the
   exact-id dedup removes the replay/live overlap class regardless of which
   emitter dominates.
