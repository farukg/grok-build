# Wave B — lane brief 6: background-thing visibility + zombie prevention

Author contract. Map (READ FIRST): docs/fork/handoff/2026-10-24/wave-b/visibility-skeptic-map.md
Worktree: /home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover. Files you
own: xai-grok-shell background_tasks/notification_bridge/monitor + xai-grok-pager
agent_view/render/scheduler surfaces.

## Requirements (Faruk)

- gb-zombie-7: on a NEW grok invocation, running tasks/monitors/subagents are
  sometimes invisible, sometimes duplicated many times (all dismissible). Fix display
  breakage AND determine/prevent real zombies.

## Map findings to implement against

- Invisibility: the tasks pane is a per-CLI replay-only reconstruction
  (render.rs:992, notification_bridge.rs:343); the authoritative reconciler
  (SessionUpdate::BackgroundTasks, background_tasks.rs:35,53) is SKIPPED in
  gateway/leader mode and has no TUI consumer. Wire the reconciler through in leader
  mode and render it as the authoritative source (replay remains as history).
- Duplicates: replay/live double-apply bypasses eventId dedup for subagent lifecycle
  events (agent_view/mod.rs:781); per-restart re-arms mint new ids
  (scheduler/actor.rs:199,861). Extend eventId dedup to ALL background-thing
  lifecycle events and key re-arm reconciliation by stable identity, not fresh ids.
- Zombies: kill_on_drop only fires in a live process; a SIGKILL'd leader orphans task
  process groups. Add a leader-startup sweep (e.g., pidfiles/process-group registry
  with bounded ownership check) that reaps orphans of dead leader generations.

## Constraints

Author-only (no cargo/git/build/subprocesses); real-entry-point tests (sibling
*_tests.rs) for: leader-mode reconciler visibility, eventId dedup across replay+live,
re-arm identity stability, startup sweep decision logic (pure parts; no real process
kills in tests). Code rules as in the other lane briefs. Write your report to
docs/fork/handoff/2026-10-24/wave-b/lane-report-visibility.md. Reply ≤10 lines.
SUBAGENTS_ALLOWED: no; work silent; never stop silently.
