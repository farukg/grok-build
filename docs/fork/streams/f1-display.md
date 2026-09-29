# F1-display — every message kind has a real one-line form

Branch: `grb/f1display` (based on `main` `c5c2c850`).

## Goal
Every message kind can be shown as a one-liner (collapsed) or fully open (expanded), chosen per
kind in the F7 "Message Display" section (default per kind) and per entry. The one-liner must be
a meaningful summary (tool label + key detail), one row at every width. Design:
`docs/fork/design/f6-f7.md` §3 and wave A3.

## Already on main
`MessageKind` (exhaustive, `Stub` internal and not a settings kind), `DisplayForm {Collapsed, Expanded}`,
`DisplayMode {Collapsed, Truncated, Expanded}` (Truncated is a transient renderer state),
`DisplayDefaults` (holds only explicit user choices; empty by default), `EntryForm {FollowingKind, Override}`,
`ScrollbackState::apply_kind_default` precedence (explicit producer modes survive; unset kinds keep
the block's own policy), and the pipeline guarantee in `RenderBlock::output` that Collapsed output
is truncated to one row.

## State (2026-09-29)
The F7 Message Display section is on `main`. The renderer WIP below was dropped: it mostly contained the changes
`reviews/f1display.md` rejects. What remains is the real goal: audit each kind's collapsed first line and add golden tests.

## Old state of the branch
- `d2a17cde` wip per-kind collapsed renderers; `2e4e5bf1` separates explicit choices from the row
  presentation default (`DisplayDefaults::row_form`, used only by the sidebar row), keeps Stub out
  of settings; `d815339c` restores upstream transient fold policies. Not compiled.
- Review rules that still apply: `docs/fork/reviews/f1display.md`.

## Open review points on the current diff (check each)
- Several blocks switch `is_foldable()` to `true` unconditionally (edit, list_dir, other, read,
  sent_message, session_event) and `subagent.rs` to `false`; `thinking.rs` changes its default
  from Truncated to Collapsed and drops running-state cycling; `read.rs` changes
  `next_fold_mode`. These change upstream fold behavior; keep upstream semantics unless the design
  requires the change, and then say why in the commit body.
- The added `if ctx.mode == DisplayMode::Truncated { take(1) }` in many tool renderers conflicts
  with Truncated being the streaming preview; the one-row guarantee belongs to Collapsed and already
  exists in the pipeline. Remove these unless a failing case proves them necessary.
- `CachedOutput` gains a `mode` key: keep only if a test shows stale output without it
  (mode changes already call `invalidate_cache`).
- Remove doc-comment churn on code that did not change.

## Next steps
1. Merge `main`; resolve the review points; improve each block's collapsed first line only.
2. Golden buffer tests (design A3): very narrow widths, all kinds, live → finished, failed
   subagent, manual per-entry override.
3. `cargo test -p xai-grok-pager` green, PR into `main`.
