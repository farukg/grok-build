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

## Thinking section boundaries and chronological order (2026-10-26)

The Responses stream preserves each reasoning delta verbatim and emits a paragraph
separator between distinct summary/content sections. `responses/reasoning.rs` tracks
item id, output index and summary/content index, observes summary-part added/done
and text-done events, and avoids separators for empty or repeated lifecycle events.
Shell `sampling_events.rs` and pager `acp/tracker.rs` retain whitespace-only reasoning
deltas so spaces and paragraph separators reach the rendered text and stored updates.
The shell's `agent/update_chunk_merge.rs` flushes at a `streamStartMs` change instead
of combining chunks from separate streams under the first stream's metadata.

The pager closes the current Thinking block when a tool call starts or a new transport
stream begins. Subsequent reasoning appends at its actual position after intervening
tool rows, through the same tracker in live and replay paths. Chunks within one section
and stream continue to append to their existing block. Explicit Left-collapse and
manual fold toggles choose a genuinely collapsed one-line state even while Thinking
is streaming. The automatic initial truncated state remains available. Further
chunks and finish preserve a manually collapsed block; the regression source is
unexecuted.

The reported sessions are `01a16ff9-184a-7e81-8853-cddd49c02952` (glued sections) and
`01a17063-984b-7c23-97a6-15036966c8ae` (correct display), both reported with
`sigma/openai/gpt-6.1-sol`, medium reasoning effort. Raw SSE evidence is unavailable;
the source defects and regression fixtures establish the repair scope, while causal
attribution for those specific runs and runtime acceptance remain open. Added sampler,
shell and pager regression tests are unexecuted. These changes are local source work.

## Next steps
1. Audit each kind's collapsed first line and improve it in the block's own renderer only; keep upstream fold semantics unless
   the design requires the change, and say why in the commit body.
2. Golden buffer tests (design A3): very narrow widths, all kinds, live → finished, failed subagent, manual per-entry override.
