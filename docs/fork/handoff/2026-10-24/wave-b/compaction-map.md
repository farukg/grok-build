# Compaction lane defect map (gb-compact-run-3, gb-compact-sub-4)

Scope: read-only source map of `/home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover` (branch `grb/owner-takeover`). Line numbers current as of HEAD `c96a1262`. Confidence per chain stated inline.

## 1. Input submission paths

- `/compact` is ALWAYS a local queue row, never an immediate send:
  - `crates/codegen/xai-grok-pager/src/slash/commands/compact.rs:28` — `run` returns `CommandResult::QueueCommand("/compact")` (doc comment lines 1–2 state this contract).
  - `crates/codegen/xai-grok-pager/src/app/dispatch/prompt.rs:1092-1094` — `QueueCommand` → `agent.session.enqueue_command(...)`; `crates/codegen/xai-grok-pager/src/app/agent.rs:979` pushes a `QueuedPrompt { kind: Command }` into the client-local `pending_prompts`.
- The double-enter user-message queue: plain prompts submitted while a turn runs go straight to the SHELL's server-owned queue:
  - `crates/codegen/xai-grok-pager/src/app/dispatch/prompt.rs:1003-1009` (`PassThrough` path sends) → shell `SessionCommand::Prompt` arm `crates/codegen/xai-grok-shell/src/session/acp_session_impl/run_loop.rs:744-825` → `queue_input` `crates/codegen/xai-grok-shell/src/session/acp_session_impl/prompt_queue.rs:113` pushes into `state.pending_inputs`; `maybe_start_running_task` (`crates/codegen/xai-grok-shell/src/session/acp_session_impl/notification_drain.rs:119-145`) starts it as soon as `running_task` is free. That is the "queued and sent ASAP" contract user messages already enjoy.
- Local drain of the client queue (commands incl. `/compact`, and idle-drained prompts): `maybe_drain_queue` `crates/codegen/xai-grok-pager/src/app/dispatch/queue.rs:315`; first gate at queue.rs:328 `if !agent.session.state.is_idle() { log_blocked("turn_running") }`. Re-kick happens on turn end (queue.rs:1304 `maybe_drain_queue_and_note_peek` after `PromptResponse`).
- When a `/compact` row finally drains, queue.rs:580-625 maps the `Command` entry to `Effect::Compact` and sets `AgentState::CommandRunning { Compact }` (queue.rs:617 `start_command`); `crates/codegen/xai-grok-pager/src/app/effects/mod.rs:1760-1779` issues the ext request `x.ai/compact_conversation` and holds it open until `TaskResult::CompactComplete`.
- Shell side: `crates/codegen/xai-grok-shell/src/extensions/memory.rs:336-393 handle_compact` — admission via `operation.compact.try_lock_owned` (memory.rs:345, returns `compact_busy` on contention), sends `SessionCommand::CompactSession` (memory.rs:357), then `FlushComplete` (memory.rs:368-380) with a 30 s timeout (memory.rs:385). `run_loop.rs:1492-1500` documents `FlushComplete` as a true sync barrier — it waits for the running turn task to settle.
- Compaction start itself never checks the turn state: the `CompactSession` arm `run_loop.rs:1204-1226` spawns `run_compact` (`crates/codegen/xai-grok-shell/src/session/compaction.rs:605-620`) CONCURRENTLY with any running turn. There is no "reject while running" anywhere; the "cannot submit" symptom is purely the client-side idle-only drain gate (queue.rs:328).

## 2. gb-compact-run-3 — incoming input aborts a running compaction

Causal chain (confidence: high for the mechanism, medium for which concrete trigger Faruk hit):

1. `/compact` drains → `CommandRunning` + ext request open (effects/mod.rs:1760).
2. Any incoming input that routes through a turn cancel reaches `cancel_running_task`, whose body unconditionally aborts an in-flight compaction: `crates/codegen/xai-grok-shell/src/session/acp_session_impl/cancel.rs:412-413` — comment "Abort in-flight `/compact` or auto-compact generation" → `self.compaction.cancel.request_cancel()`. Callers include:
   - `cancel_turn_for_send_now` (cancel.rs:132-176) — fired whenever a prompt arrives with `send_now` (run_loop.rs:811-820), i.e. exactly the "queued message gets sent ASAP / interject" path;
   - every pager cancel gesture `Effect::CancelTurn` (turn.rs, dashboard.rs below).
3. `request_cancel` resolves the select in `run_compact` (compaction.rs:611-617) → `CompactFailure::cancelled_error()` → pager renders `CompactionCancelled` (`crates/codegen/xai-grok-pager/src/app/dispatch/prompt.rs:2060-2066`). So a mere queued/interjected message or wake-driven cancel kills the compaction — no distinction between "manual explicit abort" and incidental input.
4. Manual explicit abort that DOES exist today: Esc/Ctrl+C — `crates/codegen/xai-grok-pager/src/app/dispatch/turn.rs:107-109` (compact branch beats wake early-return) and `turn.rs:238-239` `cancel_and_arm(CancellationScope::Compaction, …)`; dashboard stop `crates/codegen/xai-grok-pager/src/app/dispatch/dashboard.rs:2079-2081`. These use the same channel as (2), so the shell cannot tell them apart.
5. Secondary abort site: `run_loop.rs:1212-1220` — the `CompactSession` spawned task cancels itself when `respond_to.closed()` OR the ChildManagement cancel token fires (`child_management.rs:116 cancel_management`); `run_loop.rs:2306-2307` cancels compaction on `Shutdown`.

Gap vs contract: compaction survival must not be coupled to turn-cancel (`cancel.rs:413`); only `CancelScope::Compaction` gestures (Esc/Ctrl+C/dashboard stop, i.e. `cancel_trigger_hint` of user-gesture origin) may call `compaction.cancel.request_cancel()`.

Minimal-change outline: thread an explicit `abort_compaction: bool` (or `CancelTrigger`-keyed policy) through `CancelOptions` into `cancel_running_task`; default false for `SendNow`/wake/queue-driven cancels, true only for the pager's compaction-scoped gestures; keep `respond_to.closed()` and `Shutdown` as legitimate cancellers. Owners: `xai-grok-shell/src/session/acp_session_impl/cancel.rs` (decision), pager `turn.rs`/`dashboard.rs` (gesture labeling — already labeled via `CancellationScope::Compaction`/`cancel_trigger_hint`). Invariant: a compaction that has begun commit (`is_committing`) is never cancelled by anything but manual scope (already half-encoded at run_loop.rs:1213-1215; extend to cancel.rs).

## 3. gb-compact-sub-4 — compaction/messages with a running subagent hang

Causal chain (confidence: high for `/compact`, medium for "user messages hang forever"):

1. With a running subagent the parent pane stays `TurnRunning` (foreground await) → `maybe_drain_queue` gate queue.rs:328 blocks forever as long as the await runs; there is NO server-side enqueue path for commands — `QueueCommand` is client-local only (compact.rs:28 → prompt.rs:1093 → agent.rs:979). User prompts have the shell queue (§1); `/compact` has none. This asymmetry is the defect.
2. Compacting the CHILD directly (running subagent session): `handle_compact` → `management_session` `crates/codegen/xai-grok-shell/src/agent/mvp_agent/child_management.rs:139-160` → `ManagementResolution::Active(handle)` for a running child (child_management.rs:186-190) → `CompactSession` → `run_compact` runs concurrently, but the handler then blocks on `FlushComplete` (memory.rs:368) which is a sync barrier behind the child's running turn (run_loop.rs:1492-1500) → 30 s timeout "committed compact flush exceeded 30 seconds" (memory.rs:385-390) or a request that appears hung — instead of being enqueued behind the child's turn.
3. "Messages hang in the queue forever": shell-side `state.pending_inputs` are only promoted by `maybe_start_running_task` (notification_drain.rs:119-145), which returns early while `running_task.is_some() || finalization_gate.is_active()`. If the foreground subagent await keeps `running_task` live (or a finalization lease is never released), nothing re-kicks the queue — no timer, no wake subscription. Medium confidence: the exact "forever" repro (which lease/await never settles) is not pinned by static reading; needs a runtime trace (`qtrace` markers `server_start_blocked` notification_drain.rs:139-148 exist for exactly this).
4. Workaround that exists: `SendPromptNow` (double-enter / force-interject on the queue row) promotes a queued `/compact` immediately by CANCELLING the running turn — `crates/codegen/xai-grok-pager/src/app/agent_view/queue.rs:1591-1650` (`send_now_on_queued_compact_replaces_the_turn_with_the_command`) — i.e. today the only ASAP path destroys the turn (and via §2 also any compaction).

Gap vs contract: a queued `/compact` (and queued messages) must reach the shell immediately and run when the current turn/subagent await settles, without cancelling anything. Minimal-change outline: (a) give `QueueEntryKind::Command` a server-side enqueue: route `/compact` through the shell queue like prompts (a new `SessionCommand`/`InputOrigin` for administrative commands promoted by `maybe_start_running_task` after the running task), or (b) in `handle_compact`, when the target session has a running task, register the compact as pending and execute on turn completion instead of blocking on `FlushComplete` (owner: `extensions/memory.rs` + a completion hook in `turn_end.rs`/`notification_drain.rs`). Either way the 30 s flush timeout must not be the contract for a busy session.

## 4. CI failure cross-check

`child_prompt_tests::finished_coordinator_child_compacts_persisted_checkpoint_without_parent_delivery` (bc85 log lines 1237-1254): panic `NotTerminal { subagent_id }` from `durable_resume_source().unwrap()` near test line 512/518 (`crates/codegen/xai-grok-shell/src/agent/mvp_agent/tests/child_prompt_tests.rs:425ff`, CI revision line-drifted). `NotTerminal` is produced at `crates/codegen/xai-grok-shell/src/agent/subagent/mod.rs:1613-1618` when the child's persisted `meta.json` still has `SubagentMetaStatus::Running`. That is a terminal-status write racing the returned spawn result at child completion — a persistence/lifecycle ordering bug on the same child-management surface, NOT the queue-stall of gb-compact-sub-4 (no queue or drain involved). Adjacent owner (`child_management.rs` / `subagent/mod.rs` meta status write), distinct defect.

## 5. Queue pane perception

`crates/codegen/xai-grok-pager/src/views/queue_pane.rs`: renders BOTH queues — client-local `pending_prompts` rows (`QueueRowOrigin::Local`, queue_pane.rs:39) and server-authoritative `shared_prompt_queues` rows (`QueueRowOrigin::Server`, queue_pane.rs:41; merge in `sync_from_merged` queue_pane.rs:500). A locally parked `/compact` shows as a `#N /compact` row that never disappears while the agent runs (drain blocked, queue.rs:328) — exactly the visible "hangs forever". Server rows for queued user messages are hidden while running via `visible_held_server_row` (queue_pane.rs:22-27), so user messages *look* delivered while `/compact` visibly sits — amplifying the asymmetry perception. The pane reflects the same queues truthfully; the defect is upstream (§3.1).

## Evidence pointers

- Lane notes: `/home/fg/projects/org/forks/grok-build/docs/fork/handoff/2026-10-24/compaction-pty/` (`cancel-review.md`, `notes.txt`, `ci-*.log`, positive PTY `pty-success-a0cbf7c4.log` covering child-compact + Ctrl+C cancel + reopen).
- `goal-plan.md` items 5, 49, 106, 140 (real PTY compaction-cancel path already proven); remaining open items 13/135 cover compaction completion.
- CI failure fixture: `/home/fg/projects/org/forks/grok-build/docs/fork/handoff/2026-10-24/ci-c8-fixtures/bc85-failed-ci.log:1237-1254`.

## Gaps / unknowns

- Which concrete incoming input (user interject vs channel notification vs wake) Faruk observed aborting a RUNNING compaction is not proven from source; all go through cancel.rs:413, so the mechanism holds regardless (medium confidence on trigger, high on site).
- The exact "forever" repro for queued messages with a running subagent (never-settled `running_task`/finalization lease vs. pager pane never returning to Idle) needs a runtime `qtrace` trace; both gates exist (queue.rs:328, notification_drain.rs:130).
- Whether the parent's foreground subagent await updates the pager agent state to Idle on child completion in all paths was not fully traced (medium confidence).
