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

## Historical state before the successor repair
The earlier per-kind renderer WIP was dropped: it mostly contained the changes `reviews/f1display.md` rejects (they change upstream
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

## Pure renderer successor repair (2026-10-27; local source only)

The predecessor's complete claim was refuted by source/API evidence. F1 restores borrowed
`Option<&str>` filtering and Workflow's shared-marker phase trail. F4–F8 remove the newline-only
expanded gates, wrap long objectives/descriptions/errors/phases, bound the whole compact Subagent
line, use Injected's actual available width, and consolidate WebFetch/WebSearch/UseTool content
boxes at `scrollback/blocks/tool/body.rs`. Wrapped body selection excludes presentation indentation
and retains original source whitespace plus soft-wrap joiners. F9 extends native glyph headers to
Skill, WebSearch/X, integration discovery, Edit/create, command labels and lifecycle rows.
All compact entries now reach EntryRenderer's existing timestamp/date owner when enabled and
supported by available width; full message timestamp behavior remains unchanged.

Canonical BgTask stdout (F2) and Child-transcript output (F3) now have local source integration
through their assigned state/viewer owner, including source generations and height invalidation.
The implementation remains partial and unexecuted; the specific remaining gates are below.
Local preambles remain context, not canonical output. The separate F7/Thinking/manual-override owners are unchanged.
Regression sources are authored through RenderBlock output and EntryRenderer buffer rendering,
including exact wrapped selection text, Unicode/narrow budgets, long one-line expanded text,
glyph identity and compact timestamp/date hover. Everything is unexecuted: no build, check,
test, formatter, Git mutation, push, restart or visual capture was performed. Large-body cache,
PTY light/dark/three-accent and AppView live/replay acceptance remain open. Shared wrapping still
materializes one physical source line at a time; extremely long single-line Truncated input is
not a bounded-row wrapping proof. Full report: `/tmp/grok-build-lead-20261027/results/history-renderers-repair.md`.

Additional source blocker: the existing shared `render/wrapping.rs::word_wrap_line_with_joiners`
clips raw lines beginning with `|`/box-table markers and repeats `│ ` quote indentation as
selectable content. Its owner is outside this task's exclusive writes. The authored
`raw_table_and_quote_markers_are_content_not_layout` oracle retains exact source expectations;
it is unexecuted and source-predicted red until the lead authorizes the shared wrapping owner.
F4/F7 therefore remain partial, not accepted. The required question and exact owner evidence
are in the successor report; no input-marker masking or parallel wrapper was introduced.

## Canonical inline sources (2026-10-27; partial local source)

`app/agent.rs::{set_bg_task_stdout,append_bg_task_stdout}` updates the existing canonical
stdout and its view-owned render revision. The ACP stdout route, monitor append, demotion,
completion/tombstone and snapshot owners feed that revision path. `session_inline_output.rs`
resolves Subagent rows by real session identity plus the validated ChildLink; it never treats
labels or delegation metadata as a transcript. Expanded-demand cold reads reuse native
`replay_on_open`, outside block output, without session/load, prompt, resume or queue commands.
Empty-running, empty-terminal, loading, unavailable and cyclic sources are distinct passive
states. Unknown cold rows are not advertised as already having a foldable body.

`scrollback/entry.rs` retains the existing rendered-output cache and shares output-tree handles.
Child width/binding/source keys and weak revision dependencies drive dirty-leaf replacement in a
persistent entry tree; ordinary streaming updates do not clone or walk the complete transcript.
Initial/cold, structural and width changes still build the child index and may populate all its
Entry caches; bounded-viewport cold work and performance are not proven. The normal AppView
agent paint settles new source demands before publishing the frame and retains post-flush data.
Only changed source layouts need an additional paint; stateful media/overlay acceptance is open.
Selection paint and explicit reconstruction use the same tree, joiners, remapped ranges and
sparse boundaries. Existing preambles, Enter paths, glyph/time chrome and Thinking modes remain.

Unexecuted regression source is in `app/session_inline_output_tests.rs` and
`app/acp_handler/background_inline_tests.rs`: canonical/wire stdout, equal-length replacement,
empty/running/final/replay, widths, real live/cached/cold/nested Child content, passive channel
silence, selection ranges, unavailable/loading identity and cycles. The full-tail oracle beyond
65,535 rows retains its exact tail expectation. EntryRenderer desired height, skip, estimates,
layout height and logical-search maps now use `usize`, without a terminal-coordinate tail cap.
The physical renderer still accepts `u16` Rects. Direct minimal-terminal consumers were migrated:
`commit.rs` and `full_view.rs` batch full output into physical buffers; `live.rs` retains virtual
tail coordinates; `overlay.rs` bounds only screen sizing; `reprint.rs` keeps its existing explicit
reprint policy. Existing small-buffer test fixtures explicitly convert their physical heights.
New `wrappers/virtual_geometry_tests.rs` covers real rows, search mapping and native soft-wrap
continuations above 65,535. Fold regressions cover empty/loading collapse, cold intent and late
source arrival. All regression source remains unexecuted; no build/runtime acceptance is claimed.

The whole-block BgTask/Subagent dispatcher now consumes `SessionViews::inline_copy_text` for
both sources, preserving the originating owner/binding/EntryId and reporting unavailable/empty
output instead of copying metadata. Existing F6 y dispatch reaches this same owner. Explicit
collapse no longer depends on body existence; true terminal-empty sources remain non-expandable.
Remaining gates: aggregate `u16` range identity and compound persisted selection anchors,
arithmetic-overflow representation, stable part-based viewport anchors, viewport-lazy cold/resize
projection, explicit cold Copy, native Child leaf/media parity and Dashboard/Peek sources.
The native passive Shell storage reader loads and prepares the entire file synchronously
(`session/storage/replay.rs:301–339`); no bounded continuation API exists at that owner. Its
extension is outside the current pager partition and requires authorization before cold-work
closure. F2/F3 are not marked complete. Decision and capability mapping:
`governance/sigma/context-modules.d/500_work.d/areas.d/grok-build-fork.d/plans.d/grb-canonical-inline-sources.adr.md` (outside this repo).

## Next steps
1. Close the F2/F3 copy/input seams, unbounded composite geometry and cold-projection cost; preserve native glyphs, full wrapped text, selection joiners and compact timestamp widths.
2. Golden buffer tests (design A3): very narrow widths, all kinds, live → finished, failed subagent, manual per-entry override.
