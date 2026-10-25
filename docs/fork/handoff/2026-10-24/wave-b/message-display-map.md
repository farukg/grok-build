# Message-display map: gb-msgdisplay-11 (a) F7 defaults on NEW entries + (b) collapsible assistant messages

Codebase (current truth): `/home/fg/projects/worktrees.d/org/forks/grok-build@owner-takeover`
All source paths below are relative to `crates/codegen/xai-grok-pager/` inside that worktree unless absolute.
Read-only inspection; no builds, no edits.

## 0. The F7 "Message Display" model (what exists)

- Settings model: `src/scrollback/block.rs:187-216` — `DisplayDefaults { forms: Vec<(MessageKind, DisplayForm)> }`
  with `get/shown/set/remove`. One `DisplayForm` (`Collapsed` | `Expanded`, `src/scrollback/types.rs:63-69`) per
  `MessageKind` (`src/scrollback/block.rs:25-49`, 24 kinds). No visibility/masking axis — the F7 rows configure ONLY
  the collapse form per kind.
- Per-kind built-in start form: `MessageKind::starting_form()` `src/scrollback/block.rs:147-175`
  (AgentMessage/System/SessionEvent/ContextInfo/MemoryCapture/Stub → Expanded; everything else → Collapsed).
- F7 UI: `src/app/agent_view/context_sidebar.rs:33-54` (`DISPLAY_SECTION = SectionIdx(2)`, `RowTarget::Display(kind)`),
  rows built from `MessageKind::settings_kinds()` (`block.rs:87-112`); toggle gesture at
  `src/app/agent_view/context_sidebar/commands.rs:190-194` (single row) and `:238-265` (bulk gesture, uses
  `bulk_kinds()`, `block.rs:119-144` — user/assistant excluded from bulk by design).
- Live application of a changed default to on-screen entries: `ScrollbackState::set_kind_default`
  `src/scrollback/state/mod.rs:422-436` — sets `display_defaults`, then rewrites `display_mode` of every
  existing entry of that kind whose `entry.form == EntryForm::FollowingKind` (`EntryForm` at `block.rs:180-184`,
  `entry.form` field at `src/scrollback/entry.rs:76`, default `FollowingKind` at `entry.rs:160,190`).
- Persistence: `src/app/agent_view/context_sidebar/persistence.rs` — gesture diff → `Setting::KindForm` (:84-118),
  restore at view start / session rebind via `restore_f7_panel_settings` (:135-146) → `set_kind_default` /
  `clear_kind_default` (`src/scrollback/state/kind_default.rs:8-12`, which deliberately touches only future blocks).
  Restore entry points: `src/app/agent_view/panel_persistence.rs:157-166` (`open_panel_settings_in`) and `:196-203`
  (`sync_panel_settings`).

So the intended contract is: F7 form = the form every entry of that kind takes unless the user folded that one
entry by hand (`display_mode_pinned`, `entry.rs:78`; manual folds protected by
`respect_manual_folds` appearance flag).

## 1. Requirement (a): why NEWLY arriving entries miss the configured form

Insert-time application EXISTS and is correct:
- `ScrollbackState::push` `src/scrollback/state/mod.rs:562-614` calls `apply_kind_default(&mut entry)` at :569;
  `insert_block_before` :621-655 calls it at :637.
- `apply_kind_default` `src/scrollback/state/mod.rs:660-685`: guard `entry.display_mode == block.default_display_mode()`
  (a producer-chosen explicit mode survives), Edit special case, else `(_, Some(form)) => DisplayMode::from(form)`.
  Every production insertion path (`push_block` callers across `src/app/acp_handler/*`, agent messages at
  `src/app/agent_view/panes.rs:1476,1511,1529`) flows through `push`, so fresh entries DO consult the config at insert.

The defect is that two LATER lifecycle sites override the mode the config installed, without consulting
`display_defaults`:

1. **Finish snap — `ScrollbackState::finish_running_with_time`** `src/scrollback/state/mod.rs:1200-1203`:
   `else if let Some(mode) = entry.block.finished_display_mode() { if entry.display_mode != Collapsed { entry.display_mode = mode } }`.
   - `ReadToolCallBlock::finished_display_mode() = Some(Collapsed)` (`src/scrollback/blocks/tool/read.rs:550-552`):
     an F7 "Read → full" entry streams Expanded, then COLLAPSES the moment the read finishes. To the user this is
     exactly "the setting applies only until the next event / needs a re-trigger" — the config is silently undone by
     the finish transition.
   - Same pattern: `EditToolCallBlock` error → `Some(Collapsed)` (`src/scrollback/blocks/tool/edit.rs:1424-1428`),
     bash-mode `Execute` → `Some(Expanded)` (`src/scrollback/blocks/tool/execute.rs:733-741` — this one is guarded by
     the `!= Collapsed` check so an F7 Collapsed survives, but an F7-set Truncated-via-manual state does not).
   - `Thinking` sticky-mode branch at :1196-1198 likewise ignores `display_defaults`.
2. **Tool refinement reset — `ScrollbackState::replace_tool_block`** `src/scrollback/state/mod.rs:1020-1067`:
   when the arriving tool call refines the entry's kind (eager `Other` placeholder → concrete `Read`/`Edit`/…,
   `kind_changed` via `verb_group::verb_group_kind_changed` :1043) and the fold is not pinned, :1060-1066 resets
   `entry.display_mode` to the NEW block's built-in default (`block.default_display_mode()` / Edit policy) —
   `apply_kind_default` / `display_defaults` are never consulted. The entry the user just watched appear therefore
   renders with the block policy, not the F7 form, until F7 is re-triggered.

Root-cause statement: `display_defaults` is a first-class concept at insert and at F7-gesture time, but the
finish and replace transitions still resolve display mode from block-local policy alone. The kind default is the
stronger policy and must win in both transitions (respecting `display_mode_pinned` as they already do).

### Minimal-change proposal (a) — owner: `xai-grok-pager` scrollback state

- In `replace_tool_block` (`state/mod.rs:1060-1066`), route the reset through the same resolution as insert:
  build the mode exactly like `apply_kind_default` (a shared helper, e.g. lift the body of
  `apply_kind_default` into `fn resolve_kind_mode(&self, block: &RenderBlock) -> DisplayMode` used by both).
  Invariant: a kind change adopts the F7 form; a pinned manual fold still wins under `respect_manual_folds`;
  producer-explicit modes are not in play here (they were consumed at insert).
- In `finish_running_with_time` (`state/mod.rs:1192-1203`), gate the `finished_display_mode()` override on
  `self.display_defaults.get(MessageKind::from(&entry.block)).is_none()` — when the user has spoken for the kind,
  the finish transition must not snap the entry to the block's own preferred form. Invariant: pinned folds keep
  winning; per-block escape hatches whose purpose is *safety* rather than display preference (untrusted-edit
  expansion in `replace_tool_block`'s `untrusted_rising` branch :1048-1051) stay unconditional.
- Tests to add next to `state/mod.rs` tests (`replace_tool_block_execute_same_kind_preserves_mode` :2194 style):
  F7 Read→Expanded survives `finish_running`; F7 Read→Collapsed survives; refinement `Other→Read` adopts the F7 form.

## 2. Requirement (b): assistant messages are not collapsible

Causal chain:
1. Assistant text block: `AgentMessageBlock` (`src/scrollback/blocks/agent.rs`), rendered via
   `BlockContent::output` at `agent.rs:148-155` — it renders the full markdown body regardless of `ctx.mode`
   (no Collapsed branch at all).
2. `AgentMessageBlock::is_foldable() -> false` at `agent.rs:192-194` — hardcoded.
3. Fold machinery is fully generic and gates on `is_foldable`:
   - `ScrollbackEntry::toggle_fold` `src/scrollback/entry.rs:217-226` (`if self.is_foldable()`),
   - `ScrollbackState::toggle_fold_selected` `src/scrollback/state/selection.rs:279-286` (`entry.is_foldable()`),
     reachable from selection/timeline keys (`src/app/agent_view/selection.rs:1165-1176`,
     `src/app/agent_view/timeline_panel.rs:251`, `src/app/dispatch/router.rs:634`).
   So for assistant entries the toggle is a silent no-op and `display_mode` (even when F7 sets it, see §0) has no
   render effect.
4. Contrast — the pattern other kinds use (e.g. Read): `ReadToolCallBlock::is_foldable() = self.has_content()`
   (`read.rs:542-544`), `default_display_mode = Collapsed` (:546-548), three-state `next_fold_mode`
   Collapsed↔Truncated↔Expanded (:550-558); collapsed one-line rendering is per-block inside `output(ctx)`.
   `Btw` shows the two-state version (`src/scrollback/blocks/btw.rs:87-89`, header offset handled in
   `RenderBlock::markdown_body_line_offset` `src/scrollback/block.rs:1282-1296`).
5. F7 already exposes `AgentMessage` as a settings row (`settings_kinds()` `block.rs:87-112` includes it) with
   `starting_form() = Expanded` (`block.rs:153-155`), and `set_kind_default`/`apply_kind_default` faithfully write
   `DisplayMode::Collapsed` onto agent entries — the state half of the feature exists; only the block refuses to
   render it. (System `system.rs:79-81` is also non-foldable but is out of scope for this issue.)

### Minimal-change proposal (b) — owner: `AgentMessageBlock` (+ one starting_form decision)

- `agent.rs`: make `is_foldable()` content-driven (foldable iff the rendered body exceeds one visual line — mirror
  `UserPromptBlock`'s visual-line estimate, `src/scrollback/blocks/user.rs:495-505`, rather than `true`).
- `agent.rs` `output()`: honor `ctx.mode == DisplayMode::Collapsed` by rendering a one-line summary
  (first source line, markdown-stripped, as `Btw`'s collapsed header does); keep Expanded/raw/diagram paths unchanged.
- Add `next_fold_mode` (Collapsed ↔ Expanded two-state) and `markdown_body_line_offset` handling if the collapsed
  row gets a prefix.
- Decide the default once, at `MessageKind::starting_form()` (`block.rs:147`): Faruk's ask is "collapsible like
  other entry kinds" — keep `AgentMessage => Expanded` as the start form (assistant answers stay readable) and the
  F7 row + per-entry Space toggle become functional; a collapsed-by-default variant is then a one-arm change in the
  same match. `bulk_kinds()` exclusion (`block.rs:116-118`) already documents user/assistant as per-entry kinds and
  stays.
- Invariants to preserve: raw mode (`has_raw_mode() = true`, `agent.rs:188`) orthogonal to fold state; selection
  ranges (`agent.rs` tests: single logical range) must degrade to the visible line in collapsed form; the
  `finish_running` path needs no change (AgentMessage has no `finished_display_mode`).

## 3. Overlap with the request-visibility lane (collapsed injected entries)

- Injected/request entries are `SentMessage` tool blocks: `SentMessageToolCallBlock::is_foldable()` at
  `src/scrollback/blocks/tool/sent_message.rs:471-473` (`input.is_some() || presentation.detail().is_some()`),
  `default_display_mode = Collapsed` (:475-477). They already ride the SAME mechanism this map proposes to
  centralize: `DisplayDefaults` + `EntryForm::FollowingKind/Override` + `display_mode_pinned`.
- Recommendation: keep exactly one collapse mechanism — kind defaults (`DisplayDefaults`) resolve the un-pinned
  form, `EntryForm::Override`/pinning resolves per-entry user intent, and block-local policy
  (`default_display_mode` / `finished_display_mode`) is only the fallback when no kind default exists. The
  request-visibility lane should express "injected entries collapse" as either a `starting_form` arm or a
  producer-set explicit mode consumed by `apply_kind_default`'s existing guard — NOT as ad-hoc
  `with_display_mode` + `display_mode_pinned = true` at insertion sites, which would fight §1's fixes
  (finish/replace would have to special-case it). One resolution helper (`resolve_kind_mode`, §1) owned by
  `ScrollbackState` serves both lanes.

## 4. Gaps / confidence

- HIGH: insert-time config application (§0/§1 evidence is direct source).
- HIGH: finish/replace override sites (§1.1/1.2) — both verified line-level; Read→Collapsed-on-finish is the
  concrete reproduction of Faruk's "requires re-trigger" perception.
- MEDIUM: whether Faruk's (a) also covers kinds beyond Read/Edit-error (e.g. Thinking sticky mode, :1196-1198);
  the same one-line gate fixes all of them.
- LOW/unverified: exact keybinding for fold on the chat surface (Space vs. others) — dispatch paths found
  (`selection.rs:1165-1176`, `router.rs:634`) but not traced to key names.
- Not in scope here: `System` non-foldability (`system.rs:79-81`), F7 persistence layer-file details
  (`src/app/panel_settings/layer_file.rs`).
