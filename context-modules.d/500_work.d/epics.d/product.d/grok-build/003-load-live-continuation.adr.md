# Load reconstruction does not terminate admitted live work

## Context

Fresh-load and reconnect finalization finished the same tracker that accepted real live action updates. Finishing historical reconstruction therefore erased tool, thinking, hook, retry and compaction owners and their turn clocks. Re-adopting runningPromptId cannot reconstruct those owners. The recoverable dirty-source baseline is /tmp/grok-load-live-owner-baseline, with its original HEAD recorded there.

## Decision

Factor the existing replay-rebuilt transcript into ReplayTranscript. LoadReconstruction keeps historical reconstruction separate from the canonical live continuation after the first admitted live delivery. Finalization finishes historical tracker entries before attaching the live transcript, without finishing or restarting the live owner. A continuation remains classified after its real terminal so stale running-prompt metadata cannot resurrect it. Reconnect failure restores historical stash independently; open-window generation supersession retargets historical staging while retaining the live lane. Binding epochs reject obsolete reload finalization.

The ACP owner must route accepted Root, Child and Wake updates through with_load_delivery, separating read-only admission from mutation. Historical delivery must not mutate live turn fields outside the isolated transcript. Current already-running state requires an ordered backend-owner snapshot; historical retry or compaction replay is not such a snapshot.

## Consequences

The existing transcript IDs, replay stash, batch, workflow/todo reconstruction and activity restore owners remain in place. No polling, per-frame work, runtime read or additional startup query is introduced. Live updates whose base exists only in history still require the backend snapshot and tracker integration contract. Initial load TaskResults do not yet carry binding epochs or load generations, so same-session stale-result rejection is not complete.

## Status

Core source and regression cases are authored, not compiled or executed. ACP ingress integration and backend current-action snapshot adaptation remain external dependencies. This is not a complete action-lifecycle or reconnect-snapshot acceptance claim.
