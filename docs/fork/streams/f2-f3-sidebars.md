# F2/F3 — F6 left panel and F7 right sidebar in the session view

Branch `grb/f2f3` (from `main`). W2 and the F1-core policy API are on `main`.

## Goal (Faruk decisions, binding)
- F6 = the expanded timeline on the **left**; the rail sits flush against the panel, mirrored, and
  must look good (visual spec in the design).
- F7 = right sidebar, toggled with F7; closed means invisible (no dock, no queue above the prompt).
  Body, stacked vertically: **Dock** (the whole existing dock — workflows, subagents, tasks,
  monitors, queued — reused as it is, lightly optimized: glyphs instead of wide labels, full text on
  hover, `[↗][X]` unchanged, one line per row, no information dropped), **Context** (every category
  ON/OFF per session with warnings), **Message Display** (per kind `1 line | full`).
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
With F7 closed no dock render/snapshot/hover work. Width budgets computed once per resize or
geometry change and cached; detail popup only for the hovered/selected row; per-frame work only
for visible rows.

## State
- F7 (`ToggleContextSidebar`, `P/app/agent_view/context_sidebar.rs`): `AgentView::draw` splits the area with
  `screen_columns` (35/65 with F7 open) and draws the shared sidebar component in the right column; the
  session view draws into the center column unchanged. Rows = `switchable_categories()` with the shell's
  ON/OFF (`x.ai/session/context_policy`); space/Enter/click asks for the change, the row keeps the shell's
  last answer while `Applying`. Esc closes, Tab returns focus to the chat.
- Not yet: token estimates per row, Message Display section (needs F1-display), Dock moved into F7,
  F6 mirrored to the left with the 25/50/25 layout, Skills/MCP/Memory rows (need `Sections::Split`),
  a shell `changed` notification so other clients follow.
