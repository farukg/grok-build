# Dashboard Sessions Lane — Defect Map (gb-dash-sub-5, gb-subid-6)

Scope: read-only map of the dashboard sessions lane in
`/home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover` (branch `grb/owner-takeover`,
clean). All paths below are relative to `crates/codegen/` unless absolute.

## Summary

The dashboard is a pager-local view over two data sources: locally attached sessions
(`SessionViews`) and a leader-hosted roster (`x.ai/sessions/list` + `x.ai/sessions/changed`).
Subagent sessions do NOT appear in the leader roster today — `Summary::is_hidden()` classifies
`session_kind == "subagent"` as hidden and `merge_roster` drops hidden summaries. Subagents are
visible only as child rows/views inside a parent session's AgentView (dock subagent rows, fullscreen
takeover). The "session id loss" is not a persistence drop: the child's `acp::SessionId` equals the
durable subagent/agent id and is persisted under `sessions/<encoded-cwd>/<child-id>`. The loss is a
resolution failure: prompts to a child id only survive if the pager stamps `_meta.childOf`
(`PromptTarget::ChildOf`), which exists only when the child view carries `AgentRole::Child`; any
path that recreates the view as `AgentRole::Root` (or an attach/`-r` of a child id from the CLI)
loses that link, and the leader answers `unknown session id` / the pager startup path
`Session does not exist`.

## 1. DASHBOARD (gb-dash-sub-5 feature surface)

- Owner module: `xai-grok-pager/src/views/dashboard/` (`state.rs` ~4.9k lines is the model;
  `row.rs`/`row_title.rs`/`row_activity.rs` rows; `render.rs` chrome; `peek.rs`/`peek_tail.rs`
  live tail; `search.rs`; `usage_modal.rs`).
- Data sources (both feed the row set in `state.rs`):
  - Local attached sessions: `crate::app::session_views::SessionViews` (`dashboard/state.rs:61,71`).
  - Leader roster rows: `RosterOnlyRow` keyed by roster `session_id`
    (`dashboard/state.rs:28-35`) — "a session hosted by the leader (or a remote host) that this
    client is NOT locally attached to".
  - Roster fetch/reconcile: `xai/sessions/list` request + `x.ai/sessions/changed` broadcast;
    pager side parse `xai-grok-pager/src/app/roster.rs:80-86`, notification routing
    `xai-grok-pager/src/app/acp_handler/mod.rs:585` → `acp_handler/settings.rs:412`
    (`handle_sessions_changed`).
- "Main session" selection: rows are top-level sessions; pin/reorder identity is
  `RowPinKey::TopLevel { session_id }` serialized `top:<session_id>` (`dashboard/state.rs:99-119`),
  plus `sub:` keys for pinned subagent rows (`dashboard/state.rs:111`).
- Actions available today (must keep working for subagent rows): peek/live-tail, reply/send,
  dispatch (new session), pin, rename, stop, attach/open, close, archive, search, cycle
  (`dashboard/state.rs:216-217,4202` region; docs `xai-grok-pager/docs/user-guide/23-dashboard.md`
  and `04-slash-commands.md:23`: "peek, reply, dispatch, pin, rename, stop, attach"). Subagent
  lifecycle actions already exist on the parent's child rows: `Action::DeliverSubagent`
  (`xai-grok-pager/src/app/actions.rs:1650`, dispatch `app/dispatch/turn.rs:742`) and
  `Action::ResumeSubagent` (dispatch `app/dispatch/turn.rs:~730-740`).
- Existing subagent surfacing inside a session view (precedent, not a lane): dock rows
  `app/agent_view/panes.rs:345-380` (`dock_subagent_rows`, keyed by `child_session_id` +
  `subagent_id`, `subagents_show_all` toggle already at `panes.rs:502,524`), child views created in
  `app/acp_handler/session_notification.rs:~696` with `AgentRole::Child(ChildLink)`.

### Minimal-change proposal (feature, outline only)

- Owner: pager dashboard + leader roster.
- Smallest coherent insertion: extend the leader roster so subagent sessions become first-class
  rows — stop excluding them in `merge_roster` (`xai-grok-shell/src/agent/roster.rs:102-107`, the
  `is_hidden()` filter), instead carrying `session_kind: Some("subagent")` plus a parent link
  (add `parent_session_id` to `RosterEntry`, `xai-grok-shell/src/agent/roster.rs:48-79`, mirrored
  in pager `xai-grok-pager/src/app/roster.rs:37`). Then in the dashboard add one lane toggle
  (mirroring `dock_subagents_show_all`) that filters rows by `session_kind == "subagent"`
  (toggle-on) vs top-level (today's default); row rendering/actions reuse the existing row
  pipeline because roster rows are already generic over `RosterOnlyRow`.
- Invariants: reply/send from a subagent row must stamp `_meta.childOf` (see §3) so the leader
  resolves the child after restart; stop/deliver/resume map to the existing
  `DeliverSubagent`/`ResumeSubagent` effects already in `app/dispatch/turn.rs:730-770`.

## 2. SESSION LIST PROTOCOL

- Leader enumeration: `x.ai/sessions/list` handled at
  `xai-grok-shell/src/agent/handlers/session.rs:45-52` (`handle_roster_list` →
  `MvpAgent::build_roster`), which merges resident actor rows with on-disk summaries:
  `xai-grok-shell/src/agent/mvp_agent/session_lifecycle.rs:491-504` (`resident_roster_entries` +
  `list_recent_summaries(200)` → `merge_roster`).
- `merge_roster` (`xai-grok-shell/src/agent/roster.rs:102-156`) excludes summaries where
  `is_hidden() || is_headless()`. `Summary::is_hidden()`
  (`xai-grok-shell/src/session/persistence.rs:1457-1463`) returns true whenever `session_kind`
  starts with `"subagent"` — and `new_with_explicit_dir` stamps `session_kind = "subagent"` for
  every subagent child (`persistence.rs:3323-3332`).
- **Answer: subagent sessions do NOT appear in the leader's list protocol today** — excluded at
  `xai-grok-shell/src/agent/roster.rs:106-110` (hidden filter) + stamped hidden at
  `persistence.rs:3331`. They are reachable only per-parent via subagent notifications
  (`child_session_id` in `XaiSessionUpdate::Subagent*`, consumed at
  `xai-grok-pager/src/app/acp_handler/session_notification.rs:696,742`) and via the tools-layer
  coordinator (`list_running(parent_session_id)`,
  `xai-grok-tools/src/implementations/grok_build/task/backend.rs:418`).
- Identification: subagent id == `acp::SessionId` of the child session
  (`xai-grok-shell/src/agent/subagent/handle_request.rs:922` `child_session_id =
  acp::SessionId::new(subagent_id)`); parent link exists in payloads (`parent_session_id` +
  `child_session_id`, `xai-grok-tools/.../task/types.rs:644-645`) but NOT in `RosterEntry`.

## 3. SESSION-ID LOSS (gb-subid-6)

Causal chain (confidence: medium-high; exact user-visible string depends on entry path):

1. Assignment/persist: child session id = subagent request id
   (`agent/subagent/handle_request.rs:922-934`), persisted via `new_with_explicit_dir` with kind
   `subagent` (`session/persistence.rs:3304-3332`) under `sessions/<encoded child-cwd>/<child-id>`;
   per-attempt meta additionally under `<parent-dir>/subagents/<subagent_id>`
   (`handle_request.rs:944-958`). The id itself is durable — it is NOT rewritten on resume
   (`resume.rs` wake keeps the id; `format_resume_woken` tells the model to keep using it,
   `task/resume.rs:270-278`). Human resume mints a NEW request id only as `resume_from` source
   wrapper (`task/resume.rs:287`).
2. Send path, pager: prompt carries `session_id = child id`; survival after the leader lost the
   child actor depends on `_meta.childOf` — stamped only for `PromptTarget::ChildOf`
   (`xai-grok-pager/src/app/effects/mod.rs:5513-5519`), derived from `AgentRole::Child(ChildLink)`
   (`xai-grok-pager/src/app/agent_view/role.rs:11-16,28-33`). The doc comment states the intent:
   "A shell that lost a child session (restart) continues it from its parent's record instead of
   refusing the prompt."
3. Leader resolution: `prompt` (`xai-grok-shell/src/agent/mvp_agent/acp_agent.rs:1053-1098`):
   `session_host(child)` is `None` after the child actor retired/reaped
   (`session_lifecycle.rs:520-558`: `remove_session_terminal`/`sweep_dead_sessions`) or after a
   leader restart (child registry is in-memory only). With `childOf` →
   `wake_child_with_prompt`/`continue_child_with_prompt` → `resume_subagent` recovers via the
   parent's on-disk record (`child_prompt.rs:166-243`). Without `childOf` →
   **`acp_agent.rs:1097` "unknown session id"** — the drop site.
4. Attach/resume entry paths that surface the literal "Session does not exist":
   - CLI `-r <child-id>` / headless: `LoadSession` failure →
     `xai-grok-pager/src/headless.rs:530-536`.
   - TUI startup resolution miss → `xai-grok-pager/src/app/session_startup.rs:1052-1063`
     (`resolve_existing_session` → `RemoteMissPlan::NotFound`), fed by
     `resolve_local_session`/`resolve_local_session_any_cwd`
     (`xai-grok-shell/src/session/persistence.rs:486-491,616-620`). These look the id up under
     cwd-encoded directories; a subagent whose effective cwd differs from the caller's cwd
     (worktree child, `handle_request.rs:924-933` `resolve_child_cwd`) can miss the exact-cwd
     lookup and — with remote restore disabled — lands in the strict "Session does not exist"
     arm (`session_startup.rs:1122-1134` `plan_remote_miss`).

Defect/gap sites:

- `xai-grok-pager/src/app/agent_view/session.rs:424` — views default to `AgentRole::Root`; any
  child session that enters through a generic open/attach path (roster open, restart
  reconciliation, replayed spawn whose `ChildViewFollowUp` is dropped) gets Root role → sends
  without `childOf` → `acp_agent.rs:1097`.
- `xai-grok-shell/src/agent/mvp_agent/acp_agent.rs:1090-1098` — no disk fallback for a child id
  without `childOf`; the durable parent record (`<parent>/subagents/<id>`) is consulted only via
  the parent-scoped resume (`task/resume.rs`, `durable_resume_source_for`,
  `agent/subagent/handle_request.rs:428-460`).
- `xai-grok-shell/src/session/persistence.rs:486-491` — exact-cwd resolution ignores that
  subagent cwds are derived (`resolve_child_cwd`), so cross-cwd attach depends on the any-cwd
  fallback and remote-restore policy.

Minimal-change proposal (outline, no patch):

- Owner: leader (`xai-grok-shell`). When `prompt` hits an unknown session id, consult the durable
  subagent record (search `sessions/**/subagents/<id>` or index child→parent at spawn) and route
  to `continue_child_with_prompt` even without `_meta.childOf`; keep `childOf` as the fast path.
- Pager side: when opening/attaching a session id that is a known `child_session_id`
  (`subagent_sessions` map, `app/agent_view/mod.rs:1424`), construct the view with
  `AgentRole::Child` instead of Root (`app/agent_view/session.rs:424`).
- Invariants: child ids are never rewritten; recovery must preserve the parent-scoped ownership
  check (`send_subagent_message.rs:161` "not owned by this session") so cross-parent sends stay
  refused.

## Gaps

- Exact repro of "sometimes loses the session id" not pinned to one notification path
  (confidence medium): candidates are leader restart (in-memory child registry lost), child actor
  reap (`sweep_dead_sessions`), and replay-gated `SubagentSpawned` handling dropping
  `ChildViewFollowUp` during reconciliation. Needs a runtime trace to confirm which one Faruk hit.
- `views/dashboard/render.rs:3105` references `app/agent_view/subagent_takeover.rs`, which does
  not exist in the worktree — stale comment (minor doc-current-truth defect in the touched scope).

## Next owner / next deliverable

- gb-dash-sub-5: pager+shell change per §1 outline (roster `session_kind`/`parent_session_id` +
  dashboard lane toggle); reuse `DeliverSubagent`/`ResumeSubagent` effects for subagent rows.
- gb-subid-6: shell-side disk-backed child-id fallback at `acp_agent.rs:1097` + pager
  `AgentRole::Child` on attach; verify with a leader-restart repro against a held child view.
