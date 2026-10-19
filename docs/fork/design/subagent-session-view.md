# Design: the subagent view becomes a normal session view (sibling cycling, one implementation)

Base: `main` d6831313. Paths are under `crates/codegen/`: P=`xai-grok-pager/src`, S=`xai-grok-shell/src`, T=`xai-grok-tools/src/implementations/grok_build`.
Status: read-only analysis. Nothing was built or changed.

## 1. Why sibling cycling fails in real use

**Root cause: Faruk's terminal never sends Ctrl+Alt+Left/Right. The keys that do arrive are claimed by session cycling before the takeover sees them.**

1. **The terminal rewrites the chord.** Faruk runs grok in WSL (`/proc/version`: `6.18.33.1-wsl2`) inside Windows Terminal Preview.
   - His `settings.json` (`/mnt/c/Users/admin_fg/AppData/Local/Packages/Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe/LocalState/settings.json`, mtime 2026-09-26 18:46) binds both chords:
     - `ctrl+alt+left` → `User.grokPreviousAgent`, which is `sendInput "\u001b[91;5u"` (L62-65, L148-149);
     - `ctrl+alt+right` → `User.grokNextAgent`, which is `sendInput "\u001b[93;5u"` (L69-72, L152-153).
   - crossterm 0.28.1 (`Cargo.lock` L2306) parses `CSI 91;5u` into codepoint 91 = `'['` with modifier 5 = CONTROL (`parse_csi_u_encoded_key_code`, `event/sys/unix/parse.rs` L497-560).
   - So the pager receives **`KeyCode::Char('[')` + CONTROL, i.e. Ctrl+[**. Ctrl+Alt+Right arrives as Ctrl+].
2. **Nothing upstream alters the event.**
   - `KeyboardNormalizer::rescue` rewrites only Backspace/Delete and `\x02` (`xai-grok-pager-render/src/input/keyboard_normalizer.rs` L55-90).
   - The CSI filter handles only mouse and focus fragments (P/app/csi_filter.rs).
   - The voice chord is Ctrl+Space or F8 (P/app/event_loop.rs L3580).
3. **AppView consumes the key first.** The only guard is `takeover_owns_key = tree_chord(ev).is_some() && active_subagent.is_some()` (P/app/app_view.rs L2556-2560).
   - `tree_chord` accepts only a physical CONTROL|ALT arrow (P/app/agent_view/subagent_takeover.rs L47-66), so Ctrl+[ is not a tree chord.
   - Ctrl+[ is the `default_key` of `DashboardOverlayPrev`, and Ctrl+] of `DashboardOverlayNext` (P/actions/defaults.rs L1165-1183).
   - Direct open: `registry.lookup(key, When::DashboardOverlay)` returns `Action::DashboardOverlayPrev/Next` (app_view.rs L2561-2577).
   - Dashboard-attached: the same happens in the overlay cascade (L2579-2598).
   - Either way `AgentView::handle_input_inner` → `intercept_takeover_input` (P/app/agent_view/input.rs L360) never runs.
4. **What Faruk sees.** `dispatch_dashboard_overlay_cycle` (P/app/dispatch/dashboard.rs L972-1007) acts on the root session:
   - with one root session: `SessionCycle::Unavailable` (L1013-1014), so nothing happens;
   - with two or more: the view jumps to the next root session and closes that session's takeover (L995-997).
   - M3's tests pass because they inject the arrow chord (`subagent_takeover_tests.rs` L512-532; `dispatch/tests/dashboard.rs` L5467-5485), not what the terminal sends.
5. **Siblings are collected correctly. The defect is in routing, not in the tree.**
   - Views are created eagerly for every `SubagentSpawned`, replayed ones included (P/app/acp_handler/session_notification.rs L609-690).
   - Eviction drops only the content, never the map entry (P/app/subagent.rs L853).
   - `tree_parent`/`children_of` read `ChildLink.parent_session_id` from the wire and sort by `started_at` (subagent_takeover.rs L76-107).
   - The only gap: fewer than two siblings returns a silent `Unchanged` (L137-139).
6. **Other terminals.** Terminals that do send `CSI 1;7D` reach the takeover (M1 alt_keys). GNOME's `<Control><Alt>Left` workspace binding exists in gsettings, but no compositor is running in this WSL.

**Fix (wave 1).** Tie the decision to the action, not to the physical key.
- Both key forms (Ctrl+[/] and Ctrl+Alt+←/→), plus the header ‹ › clicks (P/app/mouse.rs L236-241), resolve to the same `Action::DashboardOverlayPrev/Next`.
- Dispatch classifies the scope once:
  ```rust
  enum CycleScope { Sessions(Vec<AgentId>), Siblings { current: String, order: Vec<String> } }
  enum CycleOutcome { Switched, Alone }
  ```
  - `Siblings` applies when the active root has `active_subagent`, using `children_of(tree_parent(..))`.
  - `Alone` → toast "No sibling subagent".
- Router L1515-1516 calls `dispatch_session_cycle(app, Direction)`, which matches on `CycleScope`.
- Delete `TreeChord::Sibling` and the Left/Right half of `takeover_owns_key`. Up/Down stay raw until wave 2.

## 2. Special-handling inventory (child ≠ main)

Legit means it implements the parent link or the final response.

| # | Site | What it does | Legit? |
|---|---|---|---|
| 1 | P/app/agent_view/role.rs L8-80 | `AgentRole::Child(ChildLink)`, plus `ChildMessaging::Unaddressable`, `ViewSurface::ChildTakeover`, `ComposerRoute::Hidden` | Only `ChildLink{parent}` |
| 2 | P/app/agent_view/session.rs L466-476 | `insert_subagent_view` forces the Scrollback pane and `QueueMutation::ReadOnly` | No |
| 3 | P/app/acp_handler/session_notification.rs L456-473, L609-690 | Drops `agent_address` (`..`); child session gets `yolo_mode: true`, empty `available_commands`, forced Vim (L656), `ChildLink::unaddressable` | No |
| 4 | same file L226-235, L1453ff; P/app/acp_handler/routing.rs L102-109, L128-134; mod.rs L438ff; queue.rs L112-136, L471; background.rs L252; mcp.rs L61 | `SessionMatch::Child` feeds a **parallel notification handler** (`handle_child_session_notification`), plus child arms in the queue, background and mcp handlers | No (SSI). Only the parent-row projection (activity, tokens) is legit |
| 5 | P/app/agent_view/subagent_takeover.rs L233-490 | Bordered frame (`render_bordered_frame`), own title row, `[✗]` | No (the kind label is kept, see §4) |
| 6 | same file L494-594 | `intercept_takeover_input`: raw chords, q/Esc close (L563-572), idle Enter quotes into the **parent** composer (L573-585), effect hoisting (L590-592), `filter_child_outcome` | No |
| 7 | P/app/agent_view/child_action_filter.rs L20-110 | Allowlist (`_ => Deny`) and `hides_chord` covering ModelPicker, CommandPalette, OpenSessions, OpenSettings, OpenExtensions, ToggleYolo, SendToBackground, EditPromptExternal, CycleMode | No |
| 8 | P/app/agent_view/input.rs L360, L1059, L1080, L1106, L1211, L1241, L1337 | Intercept step 0; root-only OpenSessions, Extensions, Ctrl+P/?; `hides_chord`; SendToBackground; composer focus refused | No |
| 9 | P/app/agent_view/render.rs L647-672, L409-415, L963, L1358, L2057 | Early return into the takeover (clears header hit areas); kill/demote disabled; prompt height 0; dashboard button off; execute cue root-only | No |
| 10 | P/views/agent.rs L802, L862-867, L992; P/views/timeline.rs L71-78; P/app/agent_view/timeline_panel.rs L18, L42 | Hints by surface; timeline rail and F6 panel hard-off for children | No |
| 11 | P/app/agent_view/session.rs L545 | Cancel-latency metric root-only | No |
| 12 | P/app/app_view.rs L2379, L1993, L2214, L2239, L2271, L2300, L2737, L2877, L5125, L5473, L5795 | Resize fan-out; toast, scroll, cache eviction, tick demand and export tip redirected to `active_subagent` | No (they exist only because the child is not the active view) |
| 13 | P/app/dispatch/ctx.rs L16-22, L56-101 | `active_agent_session_id` root-only; `get_active_agent*` / `visible_agent_mut` / `active_subagent_view_mut` swap in the child | No |
| 14 | P/app/dispatch/prompt.rs L640-707 (and 200+ `agents.get_mut(&id)` in dispatch) | Send, slash, queue, settings and model resolve the **root** | No |
| 15 | P/app/dispatch/turn.rs L62-101 | Child-view cancel sends `session/cancel(child)`; without a view the child is killed | No (see §3: the shell drops it) |
| 16 | P/app/agent_view/modals.rs L23; notices.rs L114; P/app/status_line_policy.rs L41-45; external_editor.rs L250; event_loop.rs L719; selection.rs L154/205/286/760/851; P/minimal/api.rs L369; dispatch/router.rs L1659 | Feedback blocked, banner, status line, notices, selection and draft restore special-cased for the takeover | No |
| 17 | P/app/subagent.rs L600-716, L812-859; deferred_subagent_finishes.rs | Replay on open, eviction on close (the `active_subagent` guard is at L816) | Replay/evict is legit; the takeover coupling is not |
| 18 | P/app/dispatch/dashboard.rs L286-296, L995-997; app_view.rs L2816, L2863 | Overlay rearm, cycle and close force-close the takeover | No |
| 19 | P/app/mouse.rs L630, L649, L688; subagent_takeover.rs L172-212 | Row/click opens the child | Legit (link to the child) |
| S1 | S/agent/subagent/handle_request.rs L1482ff; S/agent/mvp_agent/session_registry.rs L484; session_lifecycle.rs L167 | Child actors never enter `session_registry` (only `install_resident`, agent_ops.rs L5060) | **No: the root of every shell gap** |
| S2 | S/leader/server.rs L599-627 `prune_child_route` | Drops the child's route and driver on `subagent_finished` | No (post-finish requests become unroutable) |
| S3 | S/session/acp_session_impl/prompt_queue.rs L169; hook_dispatch.rs L437 | Prompt history and human-intent hooks are skipped because `is_subagent`, not because of authority | No once humans prompt children |
| S4 | run_loop.rs L128/191; reminders.rs L834; notification_drain.rs L609; cancel.rs L482/1028; rate_limit_waits.rs L174; hook_dispatch.rs L251; prompt_build.rs L323; memory_* | Parent owns resume status and settle; own-task kill scope on the shared backend; child budget, label, system prompt | Legit (parent linkage or shared backend) |
| S5 | S/extensions/task.rs L425-470 | `x.ai/subagent/{message,resume,cancel,get,list_running}` | Legit as parent-control; not the human prompt path |

## 3. Capability matrix (child today)

- Pager actions come from the `ActionId` list (P/actions/mod.rs). Slash commands come from `P/slash/commands/*`.
- "Gate" means `gate_child_action` denies the action or `hides_chord` swallows it (child_action_filter.rs L20-110).

| Capability | Pager (child) | Shell with the child sid | Missing |
|---|---|---|---|
| Send idle / running (queue, steer, send-now per `follow_up_behavior`) | Composer hidden (`ComposerRoute::Hidden`, input.rs L1337); send resolves the root (prompt.rs L705) | `session/prompt` → `invalid_params "unknown session id"` (S/agent/mvp_agent/acp_agent.rs L1018-1021) | Composer route, per-view send, shell residency and admission |
| All slash commands (`/compact`, `/model`, `/effort`, `/rewind`, `/fork`, `/copy`, `/export`, `/session`, `/queue`, `/plan`, `/recap`, `/btw`, …) | Unreachable (no composer); palette gated | compact: `resident_handle` None → "session failed to respond" (S/extensions/memory.rs L340-350) | Same as above |
| Model switch (ModelPicker, NextModel) | Gated (`hides_chord`) | `set_session_model` → "unknown session id" (S/agent/handlers/model_switch.rs L46-48) | Residency |
| Effort | Gated (slash) | `set_session_config_option` → `handlers::config_option::apply` (acp_agent.rs L1946-1951); lookup path not verified | Residency |
| Cancel / Esc | `CancelTurn` allowed → `session/cancel(child)` (turn.rs L62-91) | **Silent no-op**: `session_handle_waiting_for_load` → None → `Ok(())` (acp_agent.rs L1854, L1871-1916). This contradicts the plan's claim that child-turn cancel works | Residency |
| Rewind (Ctrl+R, `/rewind`) | Gated | `x.ai/rewind` uses `resident_handle` (S/extensions/rewind.rs L61) → None | Residency |
| Fork, new session, export, share | Gated / root-resolved | Fork of a child sid: not resolvable by `resolve_local_session_any_cwd` (child lives under the parent `subagents/`) | Out of scope; see §6 Q3 |
| Timeline F6 | Hard-off (timeline_panel.rs L18; views/timeline.rs L74) | n/a | Remove the gate |
| Copy block, meta, viewer, links, search, folds | Allowed | n/a | — |
| Attachments, paste images, external editor | Composer hidden; `EditPromptExternal` gated | Content goes via prompt | Composer |
| Queue edit, reorder, remove | `QueueMutation::ReadOnly` (session.rs L474) | Queue commands go through `resident_handle` → None | Residency and mutable queue |
| Settings, yolo, mode cycle (Shift+Tab), SendToBackground, Extensions, Sessions picker | Gated | `set_session_mode` → no handle → `rx` error (acp_agent.rs L1918-1937) | Residency |
| Kill bg task | Allowed | Handler lookup not verified | Verify in W1-S |
| Session switcher, prev/next | Ctrl+Alt+arrows only; Ctrl+[/] cycle **roots** (§1) | n/a | CycleScope |
| Tree Ctrl+Alt+↑/↓ | Works (raw) | n/a | Registry actions |

**Shell answer.** A child is **not** a first-class ACP session.
- Child actors are spawned with `spawn_session_on_thread`, and the handle lives only in `ShellChildRuntime` (S/agent/subagent/child_runtime.rs L12-28).
- Every by-id ACP handler resolves through `session_registry` and therefore misses children.
- The only way to address a child is through `x.ai/subagent/*`.
- The leader forgets a child's route when it finishes (S2).
- **What first-class needs:** residency in the registry for the actor's lifetime, a typed host so prompt admission keeps the parent's final-response accounting, wake instead of "unknown" for finished children, and leader route retention.

## 4. Target architecture: one session view, one store, one ACP address space

**Decision (supersedes E5, M6 and S-E).** Children become top-level `AgentView`s in `AppView.agents` with `AgentRole::Child(ChildLink)`, and `ActiveView::Agent(id)` points straight at them.

S-E feared that this would duplicate routing, replay and eviction. The evidence says the reverse:
- Routing: `find_session_match` already matches `agent.session.session_id` first (routing.rs L124-127), so child updates flow through the **root handler**. That lets us delete the parallel `handle_child_session_notification` (inventory #4).
- Dispatch: the ~200 root-resolving dispatch sites (#14) and the 30 `TaskResult{agent_id}` routes become correct for children with no edit.
- Replay and eviction move; they are not copied.

Keeping children in `subagent_views` would instead require re-addressing all 225 `ActiveView::Agent(` sites, while leaving #4 in place.

This is architecture-scale (the pager store plus shell residency), but it is the smallest correct step. Anything smaller keeps a parallel view path.

**Pager types.**
```rust
// P/app/session_views.rs (new): the only owner; raw iteration is private, so each of the 92 iteration sites must choose.
pub(crate) struct SessionViews { views: IndexMap<AgentId, AgentView> }
impl SessionViews {
  fn get(&self, AgentId) -> Option<&AgentView>; fn get_mut(..);
  fn roots(&self) -> impl Iterator<Item = (AgentId, &AgentView)>;           // dashboard, M1 cycle, quit, teardown
  fn children_of(&self, AgentId) -> Vec<AgentId>;                           // start order (M3 sort), keypress-only
  fn root_of(&self, AgentId) -> AgentId;                                    // overlay attachment, dashboard row
  fn all_mut(&mut self) -> impl Iterator<Item = (AgentId, &mut AgentView)>; // resize, sleep inhibitor
}
pub(crate) enum AgentRole { Root, Child(ChildLink) }
pub(crate) struct ChildLink { parent: AgentId, parent_session_id: acp::SessionId, subagent_id: SubagentId }
pub(crate) enum SessionKindLabel<'a> { Main, Subagent { kind: Cow<'a, str> } } // from format_subagent_label (P/app/subagent.rs L968)
pub(crate) enum CycleScope { Roots(Vec<AgentId>), Siblings { parent: AgentId, order: Vec<AgentId> } }
pub(crate) enum TreeStep { Parent, LatestChild }
pub(crate) enum TreeStepOutcome { Opened(AgentId), AtRoot, NoChild }
// Action::CycleSessions(Direction) replaces DashboardOverlayPrev/Next; Action::NavigateTree(TreeStep) is bound to Ctrl+Alt+↑/↓ in the registry.
```

**Deleted.**
- Types: `ViewSurface`, `ComposerRoute`, `ChildMessaging`, all of child_action_filter.rs, `TreeChord`/`tree_chord`, `InheritedOverlay`, `draw_subagent_fullscreen`, `intercept_takeover_input`.
- Fields: `active_subagent`, `subagent_views`, `hit_subagent_frame_close`.
- Resolution: the child branches of `get_active_agent*` / `visible_agent_mut` / `active_subagent_view_mut` / `focused_subagent_kill`.
- Routing: `SessionMatch::Child`, the child arm of `resolve_target_view`, `handle_child_session_notification`.
- The overrides in `insert_subagent_view`, `takeover_owns_key`, and inventory items #9-#13 and #16.

**Routing.**
- Spawn allocates an `AgentId` and inserts the child view with its `ChildLink` (same construction as today, minus the #3 overrides).
- Every ACP update carrying the child sid lands on the child through the root handler.
- **Parent projection:** after a child update is applied, `observe_child(parent, child_id, ChildObservation)` updates `SubagentInfo` (activity, tokens). That is the one parent-side reducer, classified once (§MOD.16).
- `SubagentSpawned` and `SubagentFinished` keep arriving on the parent sid and stay parent-owned. This is the final-response and row path, unchanged.

**Effects.** A child has its own `AgentId`, so `AppView` drains its `pending_effects` directly (app_view.rs L2752) and effects carry the child `session_id`. No hoisting.

**Replay and eviction.**
- `ensure_subagent_child_replayed` and `evict_finished_child_view` (P/app/subagent.rs) take `(parent, child: AgentId)`.
- Replay runs on switching *to* a child; eviction runs on switching *away* from a finished one.
- `ChildTranscript` (NeedsReplay / MemoryOnly / …) is kept unchanged.

**Chrome.**
- The child renders through the normal `AgentView::draw`: header, composer, status line, dock, F6 timeline.
- The header shows `SessionKindLabel::Subagent{kind}` before the title (the title is the description). M11's display projection supplies the state and cause chip.
- Enter in the block viewer quotes into the view's **own** composer (viewer.rs L1078-1082, already SSI).

**Navigation.**
- `CycleSessions`: `Roots` (M1's `session_cycle`) in a root view, `Siblings` in a child view.
- The header `‹ i/n ›` renders the same `CycleScope`, so display and dispatch come from one function.
- `NavigateTree(Parent)` from a direct child opens the parent root view. `LatestChild` uses `children_of`.
- Dashboard overlay attachment follows `root_of(active)`, so overlay Esc/Ctrl+X keep acting on the attached root. Dashboard rows come from `roots()` only.

**Shell types.**
```rust
// S/agent/mvp_agent/session_registry.rs: carried by the registry entry of a resident session
pub(crate) enum SessionHost { Root, Child(ChildHost) }
pub(crate) struct ChildHost { parent_session_id: acp::SessionId, subagent_id: String, turns: ChildTurnSink /* clone of receipt_sink */ }
pub(crate) enum ChildPromptAdmission { Live(PromptTurnReceipt), Woken(PromptTurnReceipt), ContinuedAs(acp::SessionId), Refused(SubagentResumeError) }
// S/leader/server.rs
enum ChildRoute { Live, Finished }   // finished routes retained until the parent session closes
```

**Shell behavior.**
- **Registration.** The child handle is registered resident with `SessionHost::Child` right after `child_init` (handle_request.rs ~L1676). It is released before `Shutdown` (L2338).
- **By-id handlers are then unchanged:** cancel, set_model, config_option, compact, rewind, mode and queue.
- **`MvpAgent::prompt` has the one typed branch** at admission, matching on `SessionHost`:
  - `Child` registers a turn receipt with the coordinator, so the child's last turn stays the final response the parent receives, then runs the **same** `SessionCommand::Prompt` path.
  - Non-resident known child: wake with the same identity (T/task/coordinator/wake.rs L54-146; the child sid equals the subagent id, handle_request.rs L922).
  - Not in memory after a restart: `ContinuedAs` via M4's `human_resume_request` (T/task/resume.rs L225-249). The pager follows the new child.
  - Workflow child: `Refused`.
- **Authority, not kind.** S3 gates switch from `is_subagent` to `policy.authority`.

## 5. Implementation waves (no builds; disjoint files per wave; Faruk builds once at the end)

**W1: two workers in parallel.**

**W1-P (cycle fix; pager).**
- Files:
  - P/app/app_view.rs (only the L2551-2598 block)
  - P/app/dispatch/dashboard.rs (cycle only)
  - P/app/dispatch/router.rs (L1515-1516)
  - P/app/agent_view/subagent_takeover.rs (drop `Sibling`, expose sibling cycle)
  - subagent_takeover_tests.rs
  - P/app/dispatch/tests/dashboard.rs
- Changes: `CycleScope`/`CycleOutcome` as in §1, and the toast "No sibling subagent".
- Tests (through `AppView::handle_input`):
  - `ctrl_bracket_in_takeover_switches_sibling_not_session` (Char('[') + CONTROL, 3 roots, 2 children)
  - `ctrl_right_bracket_in_takeover_switches_sibling`
  - `header_next_click_in_takeover_switches_sibling`
  - `sibling_cycle_on_only_child_shows_toast`
  - the existing `ctrl_alt_right_in_takeover_switches_sibling_not_session` stays
- Optional PTY test sends raw `\x1b[91;5u`.
- Performance: runs on keypress only.

**W1-S (first-class child sessions; shell).**
- Files:
  - S/agent/mvp_agent/{session_registry.rs, session_lifecycle.rs, acp_agent.rs (prompt admission)}
  - S/agent/subagent/{handle_request.rs (register/release), child_runtime.rs, prompt_turn_receipt.rs}
  - S/leader/server.rs
  - S/session/acp_session_impl/hook_dispatch.rs
- Changes: `SessionHost`, `ChildHost`, `ChildPromptAdmission`, `ChildRoute`.
- Tests (MvpAgent handlers plus the leader `server_tests` pattern at L4839):
  - `child_sid_prompt_runs_turn_and_parent_receives_final_response`
  - `child_sid_cancel_cancels_child_turn`
  - `child_sid_compact_reaches_child_actor`
  - `child_sid_set_model_changes_child_only`
  - `finished_child_prompt_wakes_same_session_id`
  - `restarted_child_prompt_continues_as_new_session`
  - `leader_routes_request_for_finished_child`
- Performance: registry insert/remove is O(1) per spawn; a timing assertion on spawn next to `isolated_spawn_e2e`.
- Dependencies:
  - M11 touches handle_request.rs/attempt_runner, so rebase after M11.
  - The prompt_queue.rs L169 authority change waits for the /compact fix (same file). It moves to W2-B's shell companion.
  - M5's endpoint then works for child sids for free.

**W2: pager unification, five workers in parallel.**
- Starts after W1-P, M11 and the /compact fix.
- All workers code against the §4 API stated above, so they need no shared file.

**W2-A (store and role).**
- Files: new P/app/session_views.rs; P/app/app_view.rs (field, iteration sites, deletion of #12); P/app/agent_view/{role.rs, session.rs, mod.rs}.
- Changes: `SessionViews`, `AgentRole`/`ChildLink{parent: AgentId}`; delete `active_subagent`/`subagent_views`.
- Tests:
  - `roots_excludes_children`
  - `root_of_walks_nested_children`
  - `children_of_is_start_ordered`

**W2-B (routing).**
- Files: P/app/acp_handler/{routing.rs, session_notification.rs, mod.rs, queue.rs, background.rs, mcp.rs}; S/session/acp_session_impl/prompt_queue.rs L169.
- Changes: spawn inserts a top-level child; delete `SessionMatch::Child` and `handle_child_session_notification`; add `observe_child`.
- Tests:
  - `child_update_applies_via_root_handler`
  - `parent_row_activity_follows_child_update`
  - `replayed_spawn_creates_linked_child_view`

**W2-C (lifecycle).**
- Files: P/app/subagent.rs, P/app/subagent/*, P/app/deferred_subagent_finishes.rs, P/app/agent_view/panes.rs, P/app/mouse.rs.
- Changes: replay on switch-to, evict on switch-away (keyed by AgentId); row and click open = `active_view` switch.
- Tests:
  - `finished_child_evicted_on_leave_and_replayed_on_open`
  - `row_click_opens_child_session_view`

**W2-D (dispatch and actions).**
- Files: P/app/dispatch/{ctx.rs, turn.rs, prompt.rs, router.rs, dashboard.rs}, P/actions/{mod.rs, defaults.rs}, P/app/actions.rs, P/views/dashboard/state.rs.
- Changes:
  - `CycleSessions(Direction)` with `CycleScope::{Roots, Siblings}`; `NavigateTree(TreeStep)` in the registry.
  - Delete the child resolvers and `focused_subagent_kill`.
  - Overlay attachment via `root_of`.
- Tests:
  - `child_enter_emits_session_prompt_for_child_sid`
  - `child_slash_compact_emits_compact_for_child`
  - `child_model_switch_targets_child_sid`
  - `child_esc_emits_session_cancel_for_child`
  - `cycle_in_child_is_siblings_in_root_is_roots`
  - `ctrl_alt_up_from_child_opens_parent`

**W2-E (chrome).**
- Files: P/app/agent_view/{subagent_takeover.rs → deleted, child_action_filter.rs (+tests) → deleted, input.rs, render.rs, header, timeline_panel.rs, modals.rs, notices.rs, selection.rs, external_editor.rs}; P/views/{agent.rs, timeline.rs}; P/app/{status_line_policy.rs, event_loop.rs}; P/minimal/api.rs.
- Changes: `SessionKindLabel` in the header; delete #5-#11 and #16.
- Tests (render into a buffer and assert):
  - `child_header_shows_kind_before_title`
  - `child_renders_composer_and_status_line`
  - `f6_opens_timeline_in_child`
  - `child_queue_is_editable`
- Performance guard for all of W2:
  - pty_bench, paste_latency (baselines in `xai-grok-pager-pty-harness/benches/pty_baselines`) and `event_loop_stall_tests.rs`, before and after;
  - the render path is O(visible) (the child draw replaces the framed draw, one fewer layer);
  - tick demand iterates the active view plus roots, not every child.

**W3: integration (one worker, after W2).**
- Wire compatibility: the shell advertises `_meta.childSessions: "firstClass"` in `initialize`. The pager parses it once into `enum ShellChildSupport { FirstClass, Legacy }`, and on `Legacy` it shows a typed refusal toast asking for a leader restart.
- End-to-end tests with a fake ACP peer: `child_session_full_parity_roundtrip` (send, compact, model, cancel, rewind on one child), `parent_sees_final_response_after_human_turns`.
- The final `sig build` is Faruk's.

## 6. Risks and open decisions

**Risks.**
- **Performance.**
  - Child views already exist eagerly today, so there is no memory change.
  - The shell adds one registry entry per live child.
  - The leader retains finished child routes (bounded by the parent's lifetime, O(children)).
  - Nothing polls, and there is no startup I/O.
  - Main risk: iteration sites that would now include children (92 sites in 29 files). `SessionViews` hides raw iteration, so the compiler forces each site to choose.
- **Wire and version skew.** Faruk's leader is a long-lived `grok agent --leader-socket` process. A new TUI talking to an old leader gets `unknown session id` on a child sid. The `ShellChildSupport` capability covers this.
- **Final-response integrity.**
  - A human turn on a live child must be registered as a receipt, or the coordinator could shut the actor down mid-turn (handle_request.rs L2338).
  - The finish/admit race is covered by the wake pending queue (wake.rs L67-81).
- **Persistence and resume.**
  - Children stay under the parent's `subagents/` directory. The pager rebuilds child views from the parent replay as today.
  - The shell has no resident child after a restart, so a prompt takes `ContinuedAs` (a new sid, with history copied by M4). Wake-after-restart with the same identity remains S-L.
- **Conflicts.**
  - M11 edits subagent_takeover.rs (header chip), session_notification.rs and the pager rows, so W2 rebases after it and ports the chip into `SessionKindLabel`.
  - The /compact fix owns the shared queue path.
- **UX change.** q/Esc no longer "close" a child: main sessions have no such close. Back-navigation uses Ctrl+Alt+↑, the header parent link and the session switcher.
- **Stale plan entries (not edited: read-only task; the caller decides the healing).**
  - E5, M6 and S-E, B01/B05/B09 in grb-001, and §befund's claim that child-turn cancel works (turn.rs) are contradicted by §3/§4.
  - Faruk's WT binding (§1) becomes redundant after W1-P but stays harmless.

**Q1 (product).** Where do a child's permission and question prompts appear once the child is its own session?
- (a) Only on the child view, with a NeedsInput badge on the parent row and dock plus a toast on the visible ancestor. This is normal-session semantics, like a background main session.
- (b) Also mirrored onto the visible ancestor, which is today's routing (routing.rs L165) and is a special path.
- Recommendation: **(a)**.

**Q2 (product).** After a restart, sending to a finished child continues it as a **new** child session (history copied, the view switches to the new one). Is that acceptable for now?
- (a) Yes. Same-identity wake after a restart stays in S-L.
- (b) No: build S-L persistent wake in W1-S. This is larger and adds shell state across restarts.
- Recommendation: **(a)**.

**Faruk decision: one view for every session, whatever the entry path (binding).**

Faruk (verbatim): "ich will überhaupt nichts besonderes, sondern einfach nur das was schon da ist, das was für main sessions angezeigt wird (die view die kommt wenn man eine session im dashboard entered) auch für subagents, kein modal, gleiche view, egal woher man kommt, main und subagents immer nur die view die man sieht wenn man über dashboard kommt".

- The reference is the session view you get by entering a session from the dashboard.
- Every session shows exactly that view, with the same header, hints, keys and behavior:
  - a main session opened directly (startup, resume, switcher);
  - a main session entered from the dashboard;
  - a subagent session, whether opened from a row, a link or tree navigation.
- The only additions are the session kind label in front of the title, and for a subagent, the parent that receives its final response.
- Any remaining difference between those entry paths is a defect for W2-D/E to remove. The inventory is in `../recon/view-entry-paths.md`.
- **Direction of unification (lead, from Faruk's words; overrides the recon's "delete overlay behavior" direction).** The reference is path 1, the dashboard-entered session. So every session view gets the dashboard-entered behavior unconditionally:
  - the same key context and hint row (`Ctrl+\:dashboard │ Ctrl+[/]:prev/next agent │ Ctrl+x:stop │ …`);
  - prev/next;
  - stop;
  - the same neutral-scrollback exit to the dashboard (Esc/q/Left on an empty prompt).
- `When::DashboardOverlay`'s session actions merge into the session contexts, and `attached_agent` / `DashboardReturn::{Agent, Overlay}` stop selecting chrome or input. Remove the distinction by making path 1's behavior the only one, and do not strip behavior from path 1.
- The dashboard itself (`ActiveView::AgentDashboard`, the Ctrl+\ toggle, its own Esc cascade) stays unchanged.
- Esc still cancels a running turn exactly as it does today in path 1. Keep the existing precedence in app_view.rs L2579-2683 and verify it.

**Lead decisions (Faruk: "keine Sonderbehandlung, einziger Unterschied: es gibt einen Parent"):**
- Q1 = (a): prompts only on the child view, with a NeedsInput badge and toast like any background session.
- Q2 = (a): continue as a new session after a restart. This is M4's accepted behavior; same-identity wake stays S-L.
- Q3 = (a), in scope: fork from a child view forks the child's conversation. It goes into W3, not into a later session.
- W2 order change: W2-A (store, role and API skeleton) runs alone first, then W2-B…E in parallel on top of it, because nothing is compiled between waves.

**Q3 (product).** Fork and new-worktree started from a child view:
- (a) Fork the child's conversation into a new root session. This needs a child-sid resolver in `session/fork`.
- (b) Out of scope now: fork stays available but acts on the root, with a toast.
- Recommendation: **(a)** as a follow-up after W2. It is the "everything like main" reading.
