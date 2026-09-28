# W2 — subagent view = normal session view

Branch: `grb/w2` (consolidated; contains `main` up to `c5c2c850` + all W2 waves).

## Goal (Faruk, verbatim)
"subagent view SOLL NICHT ANDERS SEIN ALS DIE NORMALE VIEW … ich will überhaupt nichts
besonderes, sondern einfach nur das was schon da ist, das was für main sessions angezeigt wird
(die view die kommt wenn man eine session im dashboard entered) auch für subagents, kein modal,
gleiche view, egal woher man kommt"

Every main-session capability works in a child view: send, `/compact`, model switch, cancel,
rewind, F6/F7. The only additions: the session kind label ("Subagent"/"General"/"Explorer")
before the title, and the parent still receives the child's final response.
Ctrl+Alt+Left/Right cycles siblings in a child and roots in a root; Ctrl+Alt+Up opens the parent.

## Design
- `docs/fork/design/subagent-session-view.md` (§2 inventory, §4 target architecture, §5 waves, §6 decisions).
- `docs/fork/recon/w2-session-views-api.md` (API and the original list of call sites to migrate).
- `docs/fork/recon/view-entry-paths.md` (entry-path differences to remove).

Target architecture in one paragraph: `SessionViews { views: IndexMap<AgentId, AgentView> }`
(`P/app/session_views.rs`) is the single owner of every view. A child session is a normal
top-level view with `AgentRole::Child(ChildLink { parent, parent_session_id, subagent_id, started_at })`.
`AgentView.subagent_views` / `active_subagent` and the takeover module are gone.
`switch_to_agent(app, target, SwitchCause)` is the one view switch (`SwitchCause::Navigate` for
tree/sibling navigation); `Action::{CycleSessions(Direction), NavigateTree(TreeStep), OpenSession(String)}`.
Child lifecycle: `crate::app::subagent::{replay_on_open, evict_on_leave}`. Child updates go through
the normal session handler; `ChildObservation::from_update` + one `observe_child` reducer
(`P/app/acp_handler/child_observation.rs`) project them onto the parent's subagent row.
Tests attach children with `session_views::test_support::link_child`.

## State
- Nothing on this branch has been compiled since `3b0006cf` removed `SessionViews`' IndexMap-style
  accessors. At `12d10731` the pager library had 124 errors; list in
  `docs/fork/briefs/w2-compile-errors.txt`. Some were fixed after that (`fd8d8afe` adds
  `SessionViews::all()`, `ee55e777` is a partial fix snapshot). Recompile for the current list.
- Remaining legacy references in tests: `P/app/acp_handler/tests/turn_completion.rs` (27),
  `P/app/agent_view/links.rs` (19, test module), `P/app/agent_view/paste.rs` (14, test module),
  `P/app/acp_handler/tests/subagents.rs:129` (`ChildLink::unaddressable`, removed API),
  `P/app/acp_handler/tests/reconnect.rs:44` (stale comment). Search:
  `rg 'subagent_views|active_subagent\b|\.active_subagent|insert_test_child|parent_with_child|ChildLink::unaddressable' crates/codegen/xai-grok-pager/src`.
- Present behavior tests: `child_header_shows_kind_before_title`, `f6_opens_timeline_in_child`,
  `cycle_in_child_is_siblings_in_root_is_roots`, `ctrl_alt_up_from_child_opens_parent`, plus
  child prompt / compact / model-switch / Esc-cancel tests in `P/app/dispatch/tests/turn.rs`.
- Branch history is a merge of many `wip(...)` commits; it is squashed into clean commits when
  integrated into `main`.

## Next steps (in order)
1. Make the library compile: follow `docs/fork/briefs/w2-fixbuild.md` (per error group: required
   direction). Key rules: per call site decide root-only flow → `roots()`/`roots_mut()`, every
   resident session → `all()`/`all_mut()`; functions taking `&IndexMap<AgentId, AgentView>` take
   `&SessionViews`; session-id → view lookup has one owner (`P/app/acp_handler/routing.rs::find_session_match`)
   and must find child views too; calls into the deleted takeover API are replaced by the one
   open/switch path or deleted; opening a child from a subagent row (double-click, Ctrl+Alt+click,
   Enter) must keep working.
2. Migrate the remaining test references listed above; delete tests that only pin the removed
   takeover/nested-view mechanics, keep and migrate tests that pin real behavior.
3. `cargo test -p xai-grok-pager` and `-p xai-grok-shell` green (except the known failures in
   `STATUS.md`); `cargo clippy -p xai-grok-pager -p xai-grok-shell --tests` clean.
4. Tests still missing from the design, only if not already covered:
   `child_renders_composer_and_status_line`, `child_queue_is_editable`.
5. W3 (after the above, together with W1-S): pager parses `_meta.childSessions` from the shell's
   `initialize` response into `enum ShellChildSupport { FirstClass, Legacy }` (Legacy → typed
   refusal toast asking for a leader restart); end-to-end tests with a fake ACP peer
   `child_session_full_parity_roundtrip`, `parent_sees_final_response_after_human_turns`;
   fork from a child view forks the child's conversation (design §6 Q3 = a).
6. Squash into a few clean commits on a fresh branch from `main` (e.g. store+role, dispatch/navigation,
   ACP routing, views/input, test migration) and open the PR into `main`.

## Log
- 2026-09-28: handoff to cloud agents at `grb/w2` HEAD (see `git log`).
