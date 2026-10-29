# Addendum to f6-f7.md: the dock moves into the F7 sidebar

Faruk's request (verbatim, German; screenshot `docs/fork/design/screenshots/dock-above-composer.png`):
"ich will dass dieser obere Bereich in die F7-sidebar kommt, weiterhin so wie jetzt, keine Änderungen außer dass es in der sidebar in die Breite weniger Platz haben wird und evtl. ein paar elegante Lösungsideen deinerseits brauchen wird"

The screenshot shows the dock above the prompt (`P/views/dock/mod.rs`, rows from `P/app/agent_view/panes.rs` `dock_subagent_rows`):
- the header `▾ Subagents 4`;
- one row per subagent: `· Subagent <description> · <activity>`, with the right-aligned meta `resumed <model> <elapsed> · <tokens>/<window> [↗][X]`.

## Requirements
- **Shared work display.** The dock contains Workflows, Subagents, Tasks and Watchers. Todos and the composer queue are independent areas and must not belong to the dock. The current `Queued` dock section is an implementation gap; preserve queue editing and actions when separating it.
  - Use the richer `dock off` task presentation, including its colors, status information, controls and output-line badge, for top, bottom and F7 work displays as well.
  - Preserve section navigation, collapse, show-N-more, open and stop actions, keyboard and mouse behavior, and stop confirmation. Share presentation logic and source state across views.
- **The only intended difference is less width.** Faruk decided how to handle that:
  - One line per row. Two-line rows are rejected.
  - Information must not be lost: no shortening, no dropping of fields.
  - Use symbols/icons instead of text labels. Examples:
    - the kind label ("Subagent") becomes a glyph;
    - `resumed` becomes a glyph;
    - `[↗]` / `[X]` stay exactly as they are: they already are compact icons;
    - the activity ("Thinking", "Waiting for response…") becomes a state glyph;
    - the model/elapsed/tokens separators become compact.
  - Less important parts and full text appear on HOVER. The mouse-over on a row shows the full untruncated row: complete description, full model path, full activity text, tokens/window, and anything else hidden or truncated. Design the hover surface:
    - reuse an existing grok hover/tooltip/popover mechanism if one exists (search `update_hover`, `hovered`, tooltip/popover in P/);
    - otherwise the smallest clean one;
    - keyboard equivalent: the selected row shows the same detail.
  - Icons come from the existing glyph owner (`crate::glyphs`), including its ASCII fallback behavior. Invent no second glyph table.
  - The width-driven layout decision is made once per resize and cached, O(visible rows) per frame. Hover detail is rendered only for the hovered row.
- **Independent placements.** F7 hosts its own work display while open; top, bottom and off select the main-session work placement. Closing F7 closes its sidebar display. The independent queue remains available at the composer according to its own visibility and interaction rules.
- **Performance:** no new per-frame work beyond today's dock and no extra allocations per frame. The dock row cache and layout stay shared.
- Works identically in main and subagent session views.

Integrated into f6-f7.md: layout (§5), waves (§6, worker B4) and risks (§7).
