# W2 fixbuild brief (lead)

Branch `grb/w2` (contains main up to `c5c2c850` + M5).
Goal: `cargo check -p xai-grok-pager -p xai-grok-shell` (library, no `--tests`) green on `grb/w2`.

Evidence: 124 errors at `12d10731`; error list with locations in `docs/fork/briefs/w2-compile-errors.txt`
(line numbers drift as fixes land — recompile for current truth).
Nobody compiled W2 since `3b0006cf refactor(pager): classify root view iteration in SessionViews`
removed the IndexMap-shaped accessors; consumers were never migrated, and several workers left
broken edits. `SessionViews::all()` has been added since (`fd8d8afe`).

Read first: `docs/fork/design/subagent-session-view.md`, `docs/fork/recon/w2-session-views-api.md`,
then `crates/codegen/xai-grok-pager/src/app/session_views.rs`.

## Error groups and required direction

1. `SessionViews` has no `values/values_mut/iter/keys` (52 sites), `&SessionViews` not iterable
   (routing.rs:83), 41 `expected &IndexMap<AgentId, AgentView>` mismatches (dashboard.rs,
   views/dashboard/state.rs, workspace_sync.rs, event_loop.rs, task_result.rs, …).
   - Decide per site: root-owned flow → `roots()`/`roots_mut()`; every resident session (settings
     propagation, auth, theme, tick, session-id lookup) → all views. Add exactly the missing
     all-views reader(s) to `SessionViews` (e.g. `all()` beside `all_mut()`), no IndexMap
     re-export, no `Deref`, no compatibility aliases named `values`/`iter`.
   - Functions that take `&IndexMap<AgentId, AgentView>` take `&SessionViews` instead.
   - Session-id → view lookup: one owner (`acp_handler/routing.rs::find_session_match`); now that
     children are top-level views it must match child views too. Do not add a second lookup.
2. Calls to deleted takeover API: `close_subagent_fullscreen` (6), `open_subagent_fullscreen` (2),
   `a_subagent_owns_the_frame`, `intercept_root_tree_input`, `child_link_at_scrollback_row`,
   `try_open_child_from_selected_row`, `detach_child_view_content`,
   `session_notification::apply_child_view_session_event`, `ChildLink` import in
   child_observation.rs. Replace with the SessionViews equivalent (`switch_to_agent(…,
   SwitchCause::Navigate)`, `Action::OpenSession`, `subagent::{replay_on_open, evict_on_leave}`)
   or delete the call when the behavior is gone by design (no takeover). Opening a child from a
   selected/clicked subagent row must keep working (double-click / Ctrl+Alt+click / Enter) via
   the one open path.
3. Broken worker edits: `app_view.rs:5672` missing `;`/closing of the `fast` expression;
   `app_view.rs:2218` uses `scrollback` without binding (use `agent.scrollback`); `turn.rs:65`
   unbound `id`; `ctx.rs` missing `AgentId` import; `tip_seen_counts` double mutable borrow;
   `dashboard.rs:206` unsized `Path`; `subagent.rs:756` `&child_sid`.
4. `actions::Category::Agent` does not exist (defaults.rs:1167..1206, session navigation actions):
   use the existing `Category::Session`; do not add a category.
5. `SessionUpdate` type mismatch (acp vs `xai_grok_shell::extensions::notification::SessionUpdate`)
   — fix at the pager call site, do not change the shell wire type.

## Rules
- Production code only; do not touch test modules except where a test helper blocks the lib build.
  Other workers are editing `acp_handler/tests/*`, `agent_view/{links,paste,role_tests}.rs` tests
  and `agent_view/mod.rs` test helpers in their own worktrees — leave those regions alone.
- §MOD: no catch-alls on owned enums, no `unwrap/expect`, no dead code, no comments narrating
  the migration, no compatibility shims. Root-cause fixes only.
- Commit in small commits (subjects `fix(pager): …`).
- Build: `cargo check -p xai-grok-pager -p xai-grok-shell`. Fix everything visible in a log before
  the next check; do not build after every edit.
- Record progress in `docs/fork/STATUS.md` (W2 section).
