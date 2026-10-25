# Wave B — lane report 5: dashboard subagent sessions + session-id stability

Status: **implemented** (author-only pass; no cargo/build/test/git run — verification by the
integration lane is still pending).

Worktree: `org/forks/grok-build@owner-takeover`. Map:
`docs/fork/handoff/2026-10-24/wave-b/dashboard-sessions-map.md`.

## What landed

### gb-dash-sub-5 — dashboard subagent lane

- Leader roster (`xai-grok-shell/src/agent/roster.rs`): `RosterEntry` gains
  `parent_session_id`; `merge_roster` stops excluding subagent-kind summaries — they are
  emitted (and backfilled onto resident rows) as `session_kind: "subagent"` rows with their
  persisted parent link. Headless and explicitly-hidden summaries stay excluded
  (`excluded_from_roster`); the "hidden" rule now lives once in
  `Summary::is_subagent_kind()` (`session/persistence.rs`).
- Live-child reconcile (`mvp_agent/session_lifecycle.rs`): `build_roster` re-stamps subagent
  rows from the live child host — Running → `resident: true` + `resident_activity` (Working
  rows keep a "now" timestamp, mirroring root rows), Finished → `Completed`/not resident,
  host gone (reaped actor / leader restart) → dormant summary row. Parent link always from
  the live `ChildHost`.
- Pager mirror (`xai-grok-pager/src/app/roster.rs`): `parent_session_id` field +
  `SESSION_KIND_SUBAGENT` const + round-trip test.
- Dashboard lane (`views/dashboard/row.rs`): new `SessionLane` enum (`Main` default /
  `Subagents`) threaded through `build_rows_with_roster` → `build_local_rows` +
  `append_roster_rows`. Main filters subagent entries out (toggle-off = today's behavior,
  byte-for-byte); Subagents lists exactly the subagent roster entries plus locally-attached
  child views (richer data wins over the roster twin, same as the main lane). The untitled
  "New session" husk rule stays main-lane-only, so untitled children still list (cwd/id label
  fallback). Same sorting/grouping/filter/pin/reorder pipeline — no separate subagent path.
- Toggle (`views/dashboard/state.rs`): `DashboardState::subagents_lane` (default **off**,
  in-memory like section collapse), `Ctrl+A` chord (mirrors the hardcoded `Ctrl+/` search
  chord; no registry action), header `◆ subagents` chip while on (`chrome.rs`). Render,
  focusables, and telemetry consume the same rows; `cycle_order` stays Main-lane (cycling
  walks roots only).
- Docs: `xai-grok-pager/docs/user-guide/23-dashboard.md` — lane paragraph + keybinding row.
  Also healed the stale `render.rs:3105` comment (referenced the nonexistent
  `app/agent_view/subagent_takeover.rs` / `draw_subagent_fullscreen`).

Actions on subagent rows reuse the generic row surface: roster rows keep peek/attach/pin/
rename/stop/archive/delete; locally-attached child rows are ordinary `TopLevel` rows, and
`DeliverSubagent`/`ResumeSubagent` remain available from the parent's dock.

### gb-subid-6 — session-id stability

- Durable parent stamping (`session/persistence.rs`): `ExplicitSessionOpen::New` carries
  `parent_session_id`; `new_with_explicit_dir` stamps it into the child's `summary.json`
  next to the `subagent` kind (spawn site `subagent/handle_request.rs` passes
  `ctx.parent_session_id`). Parentage now survives leader restarts for both the roster and
  the error path. `Summary.parent_session_id` keeps its existing fork/remote meanings; the
  stamp never overwrites a set value.
- Child-role stamping (`app/agent_view/session.rs`): `stamp_child_role_for_session` — the
  generic open path (`app/dispatch/session/load.rs`, i.e. dashboard attach, picker, `-r`)
  stamps `AgentRole::Child(ChildLink)` when any view's `subagent_sessions` knows the opened
  id, so sends carry `_meta.childOf` and the leader's `continue_child_with_prompt` fast path
  recovers the child after restart/reap. Views no parent knows stay `Root`.
- Typed refusal (`mvp_agent/child_prompt.rs` + `acp_agent.rs` unknown-session arm):
  `UnknownPromptTarget::{ReapedChild { child, parent_session_id }, Unknown { session_id }}`
  with `wire_payload()`; the prompt path consults the durable child record
  (`find_summary_by_session_id` via `spawn_blocking`, same as `session/merge.rs`) only when
  the id is unknown and no `childOf` is present. A reaped child answers with
  `{kind: "reapedChild", sessionId, parentSessionId, message}` naming the child, its parent
  when known, and the recovery path (open parent + resume; childOf-stamped sends continue
  automatically). Genuinely unknown ids keep the byte-identical `"unknown session id"`.

## Decisions

- `Ctrl+A` hardcoded (not a registry ActionId): mirrors the `Ctrl+/` search-mode precedent;
  no chord was free without shadowing a prompt-widget binding (Ctrl+U kill-line, Ctrl+Y
  yank, Ctrl+P history-browse are all taken by the dispatch input).
- Typed error instead of silent auto-recovery at `acp_agent.rs:1097`: the brief asks for an
  actionable error naming child + recovery path; auto-recovery via the disk record remains a
  follow-up (the `childOf` fast path already recovers automatically).
- Subagent rows ride the existing roster wire — no new protocol method; old leaders simply
  never send `subagent` rows, and the pager tolerates the missing field (`serde(default)`).
- Running-child activity is reconciled after `merge_roster` rather than enumerating
  children as resident entries (`resident_ids` is root-only by design).

## Tests added (real entry points; no UI-render goldens)

- Shell `agent/roster.rs` (merge): subagent summaries → subagent rows with parent; resident
  backfill of kind+parent; explicitly-hidden stays excluded; `parentSessionId` camelCase +
  skip-when-none serialization.
- Shell `session/persistence_worktree_stamp_tests.rs`: parent stamped into the child
  summary at explicit-dir creation.
- Shell `mvp_agent/tests/child_prompt_tests.rs` (`unknown_target_tests`): payload shape for
  reaped child (names child, parent, recovery) and bare unknown; disk classification
  (GROK_HOME-pinned tempdir) of reaped-child vs unknown; `prompt` to an unknown id still
  `InvalidParams`.
- Shell `mvp_agent/tests/roster_lane_tests.rs` (new): running child → resident + live
  parent; finished child → Completed/not resident; host-less child stays dormant;
  non-subagent rows untouched.
- Pager `app/roster.rs`: subagent kind + parent link round-trip through the result envelope.
- Pager `views/dashboard/row.rs`: main lane drops subagent entries; subagents lane keeps
  exactly them (titled or not); locally-attached child listed as a local row with its roster
  twin skipped while the main lane still lists only the root.
- Pager `views/dashboard/state_tests.rs`: lane defaults off, `Ctrl+A` round-trips.
- Pager `app/agent_view/role_tests.rs` (file existed empty and undeclared; now declared):
  known child id opens with `Child` role + correct `ChildLink`; unknown id stays `Root`;
  stamped view's `prompt_target()` is `ChildOf(parent)` (the `_meta.childOf` source).

## Open questions / follow-ups

- Child transitions (spawn/finish) do not emit `x.ai/sessions/changed` deltas — the
  subagent lane refreshes on the existing roster poll cadence. Emitting child deltas needs a
  hook in the subagent coordinator (outside this lane's file ownership).
- Children spawned before the parent-stamping change have no durable parent link: their
  reaped-child error names the child and the recovery path but not the parent.
- Auto-recovery without `childOf` (route the prompt to `continue_child_with_prompt` straight
  from the durable record) is sketched in the map §3 but intentionally not implemented —
  the brief's required behavior is the typed actionable error.
- Not verified by build/tests/clippy (author-only constraint); formatting hand-checked
  against rustfmt width, but a `cargo fmt` pass by the integration lane is advisable.
