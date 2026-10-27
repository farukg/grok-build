# F2/F3 — F6 left panel and F7 right sidebar in the session view

Squashed onto `main` (the branch `grb/f2f3` is history). W2 and the F1-core policy API are on `main`. Tree navigation, gestures, row colours and saved
settings of both panels: `streams/f6-f7-trees.md`.

## Goal (Faruk decisions, binding)
- F6 = the expanded timeline on the **left**; the rail sits flush against the panel, mirrored, and
  must look good (visual spec in the design).
- F7 = right sidebar, toggled with F7; work and Todos stay available there independently of upper-display preferences.
  Queue stays independently directly before the composer; Dock enables an optional upper work/Todo projection.
  Body, stacked vertically: **Dock** (the whole existing work dock — workflows, subagents, tasks,
  monitors — reused as it is, lightly optimized: glyphs instead of wide labels, full text on
  hover, `[↗][X]` unchanged, one line per row, no information dropped), **Context** (every category
  independent Global/Session ON/OFF with aggregate warnings), **Message Display** (per kind `1 line | full`).
- Columns: both open 25/50/25, one open 35/65 or 65/35, none 100.
- Both sidebars use the shared component already on `main` (`P/views/sidebar`, commits `d13b0729`,
  `f6b45711`): fixed 1-row header like the session header's top row, fixed footer at the height of
  the empty composer, scrollable body with scrollbar.
- Same behavior in child views (they are normal session views after W2).

## Design
`docs/fork/design/f6-f7.md` (read "Faruk decision: layout" first; it overrides older width rules),
`docs/fork/design/f6-f7-dock.md`, `docs/fork/design/f6-f7-mockups.md` (+ `mockup-width-check.py`),
screenshots in `docs/fork/design/screenshots/`. Wave order: B2 geometry/input → B3 actions/ACP
bridge/persistence → B4 dock into F7 → D F6 visual rework.

## Performance (binding)
With F7 closed no sidebar dock render/snapshot/hover work; the optional upper projection remains independent.
Width budgets computed once per resize or
geometry change and cached; detail popup only for the hovered/selected row; per-frame work only
for visible rows.

## State
- F7 (`ToggleContextSidebar`, `P/app/agent_view/context_sidebar.rs`): `AgentView::draw` splits the area with
  `screen_columns` (35/65 with F7 open) and draws the shared sidebar component in the right column; the
  session view draws into the center column unchanged. Rows = grouped `all_categories()` with the shell's
  ON/OFF (`x.ai/session/context_policy`); Space asks for the change, Enter inspects and Context clicks only select/focus; the row keeps the shell's
  last answer while `Applying`. Esc closes, Tab returns focus to the chat. Sections are branches of a tree with the keys and
  gestures of `design/f6-f7.md` ("F6 outline keys, F7 gestures and saved settings").
- Dock (B4): hosted in the top rows of the F7 body (blank `DockSlot` section, painted after the sidebar
  chrome via `PendingDock`); F7 work stays available regardless of Dock or global upper-display preferences.
  The optional upper work/Todo projection follows Dock and its global visibility preferences; queue remains
  independently directly before the composer. Sidebar dock rows are capped at half the body height.
- Message Display: semantic groups contain individually controllable settings kinds. Space/click on a group sets one common value for all children, including folded ones; Mixed → Line → Full → Line. Enter opens the existing viewer. `g` selects persistent effective Display scope, independently of Context's `c`; folds use Global only. The shared kind resolver preserves explicit choices for existing/future/refined/replayed entries and legacy `injected` fallback precedence (Session child, Session parent, Global child, Global parent). See `app/panel_settings/` and `scrollback/block.rs`.
- F7 hierarchy source follow-up (2026-10-27): Context groups instructions/capabilities, conversation, continuation/recovery, notifications/activity and retained context. Display groups conversation, files/commands, research/integrations, background work, diagnostics and typed injected sources. Group Context edits call the finished authority's `request_context_change(SetCategories)` once; no direct policy writes. Every top/group fold persists globally; selection repairs by semantic identity/visible ancestor through the existing Outline. The navigation owner's `sidebar_end_is_stay` helper is retained unchanged and consumes that same tree. Regression sources are authored, not executed; build, tests, cross-client and visual acceptance remain open.
- F6 sits in the left column with the 25/50/25 layout (`e299b7f3`).
- Open acceptance: dock glyph/width optimisation and hover detail, request inspection, runtime/visual verification. Context category rows and the authority's changed notifications are authored; their runtime fanout remains unverified.

## Open: F6/F7 hang on very large sessions (reported 2026-09-29)
Opening or closing F6/F7 hangs the UI for 20-30 s in session `01a0b0d3` (~92k updates, 412 MB updates.jsonl);
small sessions are instant. A synthetic probe with up to 42k entries (long verb-group runs, 400-line outputs)
needed at most ~60 ms per frame in a debug build, so layout and outline are not the cost. Suspect: the terminal
writer stalling (`term.writer.blocked` / `term.writer.recovered` in `~/.grok/logs/unified.jsonl`, `event_loop.rs` Presenter).
Needs: those log lines and a `GROK_FPS=1` run on the large session.
