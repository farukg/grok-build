# One timestamp-width owner for inline paint and selection

## Context

The frozen old-session evidence identifies a synchronous inline paint retry in
`AppView::draw_inner`. At baseline `8f4a26ffc2f8f17299b622618691484b647a1c36`,
`EntryRenderer` reserves ten timestamp columns for every collapsed entry, but
`scrollback/render.rs::timestamp_reserved_for_block` reserves them only for
messages. A collapsed BgTask is therefore measured/painted at W-10 and then
re-cached for selection at W. Inline preparation reads that last cache width;
settling its dirty height measures W-10 again. Every subsequent paint repeats
this transition even for terminal-empty output. The shipped `7df29730` source
has the same divergent classifiers.

The recoverable baseline is that SHA plus the pre-edit files under
`/tmp/grok-paint-livelock-baseline/`. No session data or running process is part
of the replacement transaction.

## Decision

Selection calls the existing renderer's `timestamp_reserved` on the same renderer
instance used for paint. Delete the duplicate block-only classifier in the same
change. Do not change source keys, projection lifetimes, native output trees,
copy, display modes, layout retries or persistence.

Retained capabilities and verification owners:

| Capability from source | Target owner | Verification source |
| --- | --- | --- |
| Collapsed timestamps and expanded message timestamps | `EntryRenderer::timestamp_reserved` | Existing message-width oracle and new replay paint geometry oracle |
| Background-task canonical stdout, empty notices and distinct row identities | Existing SessionViews/OutputTree owners | New replayed empty/nonempty, expanded/collapsed/mixed paint and copy oracle |
| Width settlement on resize/sidebar/scroll edges | Existing layout and projection owners using the shared width | New AgentView replay/layout/draw regression |
| Deferred native child parts and passive inactive children | Existing EntryTree and passive replay owners | New deferred-window regression and existing 70,000-part regression |
| Full virtual output and source-bound selection/copy | Existing virtual geometry and selection owners | Existing full-tail, canonical copy and selection identity regressions unchanged |

The deleted function is private and had exactly one caller, now delegated to
`EntryRenderer`. No capability is removed and no compatibility path remains.

## Consequences

Paint, selection and exact measurement agree on the timestamp gutter. Fixed-source
collapsed rows no longer invalidate their projection on every frame. This removes
a redundant classifier without new per-frame allocation, history reads or polling.
Missing-source notices, native child rendering, virtual heights and unwrapped copy
remain unchanged. This is not a claim that every independent historical inline
completion gate has been closed.

## Status

Source repair and regressions authored; no build, check, test, formatter, Git
mutation, restart or runtime acceptance performed. The frozen process's exact
entry/width sequence was not read; the source establishes a sufficient repeatable
cause on its replayed BgTask path, not a new observation of that process.
