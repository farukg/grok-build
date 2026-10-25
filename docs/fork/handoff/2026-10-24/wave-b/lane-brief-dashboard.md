# Wave B — lane brief 5: dashboard subagent sessions + session-id stability

Author contract. Map (READ FIRST): docs/fork/handoff/2026-10-24/wave-b/dashboard-sessions-map.md
Worktree: /home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover. Files you own:
xai-grok-shell roster/persistence/session-lifecycle + xai-grok-pager dashboard views.

## Requirements (Faruk)

- gb-dash-sub-5: a dashboard toggle that, once activated, shows ALL OTHER RUNNING
  SESSIONS (subagent sessions) instead of the main session. Sorting, view, actions
  EXACTLY like the current main-session dashboard; assume all familiar actions then
  work for subagent sessions too. Nothing else new.
- gb-subid-6: subagent sessions sometimes lose their session id; sends then error
  "Session does not exist". Fix per map: child views created without AgentRole::Child
  (app/agent_view/session.rs:424) send prompts without _meta.childOf; after the leader
  loses the in-memory child host (restart/reap, session_lifecycle.rs:520-558) the
  prompt falls to "unknown session id" (acp_agent.rs:1097).

## Required behavior

- Roster stops hard-excluding session_kind == "subagent" (roster.rs:102-110) for the
  dashboard toggle path; RosterEntry carries parent_session_id; the pager dashboard
  gains a toggle (default off = today's behavior) that lists other running sessions
  with the same sorting/view/action surface as the main dashboard.
- Child views always carry AgentRole::Child / _meta.childOf so parentage survives
  leader restarts; a send to a reaped child surfaces a typed, actionable error (not a
  generic "unknown session id") and the dashboard reflects reaped sessions correctly.

## Constraints

Author-only (no cargo/git/build/subprocesses); tests through real entry points in
sibling *_tests.rs; enums with payloads; no unwrap/expect/panic outside tests;
exhaustive matches; minimal diffs; comments only for non-obvious constraints.
Write your report to docs/fork/handoff/2026-10-24/wave-b/lane-report-dashboard.md
(status, decisions, tests added, open questions). Reply ≤10 lines. SUBAGENTS_ALLOWED:
no; work silent; never stop silently — report blockers precisely.
