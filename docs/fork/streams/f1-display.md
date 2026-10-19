# F1-display — every message kind has a real one-line form

The F7 Message Display section is on `main`; the branch `grb/f1display` is history.

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
is truncated to one row. The chosen kind defaults are saved globally or per session (`streams/f6-f7-trees.md`).

## State
The per-kind renderer WIP was dropped: it mostly contained the changes `reviews/f1display.md` rejects (they change upstream
fold behavior, add `Truncated` special cases to many renderers and keys to `CachedOutput`). What remains is the real goal.

## Next steps
1. Audit each kind's collapsed first line and improve it in the block's own renderer only; keep upstream fold semantics unless
   the design requires the change, and say why in the commit body.
2. Golden buffer tests (design A3): very narrow widths, all kinds, live → finished, failed subagent, manual per-entry override.
