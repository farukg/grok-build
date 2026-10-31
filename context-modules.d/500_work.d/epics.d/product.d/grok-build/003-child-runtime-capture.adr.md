# Child runtime lifecycle and observation capture

## Context

The canonical graph already owns inventory transactions and evidence, but not runtime custody. Fresh admission-queue entries have no execution attempt. The message-quota target registry previously initialized ordinary targets only after Started or through a minting accessor. Existing inspection constructors could neither represent genuine pre-execution custody nor supply a real resident target. Completed-map presence alone does not prove that finishing, producer commit or publication has settled.

Recoverable author baseline: `a82c8efcedcca938996f50e8061fcc5ccb35a3ca`, with the pre-edit task subtree and existing worktree patch in `/tmp/grok-child-runtime-lifecycle-baseline/{source.tar,scoped-wip.patch,head}`. The archive includes predecessor work; it is not permission to restore over other authors' changes.

Source-derived retained capabilities:

| Capability | Retained owner | Authored verification |
| --- | --- | --- |
| Concurrent admission, FIFO, cross-session capacity, cancellation and destructor delivery | Existing SpawnQueue, now ordered keyed storage | Existing admission/queue oracles; real spawn/dequeue ordering oracle |
| Pending initialization and successful promotion | Existing start_child / Started handler | Pre-read target and actual Pending/Active capture oracles |
| Sender quotas, prospective wake identity, replacement and rollback | Existing AgentMessageQuotas and displacement transaction | Unauthorized lookup, wake rollback and completion-before-Started identity oracles |
| Actual child progress | Existing ChildControl::progress | Counted control with nonzero independent expected counters |
| Completion delivery, producer/visibility acknowledgements and parent-stop identity | Existing completion/publication pipeline | Existing oracles unchanged; no ACK-path replacement |
| Finishing, committing, publishing and blocked custody | Existing terminal_outputs plus matching completed metadata | Phase, stale-completed and missing-metadata oracles |
| Full terminal output and compact headers after output eviction | Existing completed result and strict terminal evidence types | Exact output/counter oracle, compact fact and invalid-Running-result oracles |
| Graph transactions, source revisions, audience and release fences | Existing SpawnGraph | Existing graph suite unchanged; runtime audience/release/proof oracles |

No retained capability is removed. The obsolete private completed_terminal constructor is replaced by the strict completed-inspection projection. Resident/running/pending/queued/completed inspection constructors have one implementation in coordinator_state/inventory.rs; there is no old parallel constructor path.

## Decision

- AttemptEvidence::NotStarted is positive runtime evidence, distinct from historical Legacy. Historical sources cannot assert NotStarted. Historical Spawned/Running evidence cannot carry it. An explicit no-start proof may omit the old attempt_id field; a missing attempt without a proof still means unconfirmed Legacy, never Current.
- CompletedOrigin owns either an actual attempt-start anchor or an admission-only anchor. The existing publication correlation ID and ACK payloads remain unchanged. Admission-only inventory results do not expose that correlation as an execution attempt, child session or execution start time. Their genuine terminal result is Recorded evidence from the actual Coordinator revision, not an invented Producer execution.
- Committing custody carries its own CompletedOrigin. A stale or absent completed-map entry cannot decide whether its execution started. Publishing has no full metadata payload and therefore requires a matching completed record; a missing or contradictory join fails explicitly.
- Targets are initialized at actual admission/start/never-started custody transitions in the existing registry. Ordinary promotion no longer initializes a target. The readonly typed target accessor derives the current attempt from actual custody and never invokes a minting accessor. A reserved wake target does not replace the observed completed target before actual custody transfers. Rollback restores the displaced identity; retained successful completion commits replacement even if no Started callback arrived.
- SpawnQueue uses one IndexMap as its custody store, preserving arrival order while permitting exact-ID reads. There is no second roster or target registry. The extra bounded hash/key cost replaces global queue scans for requested captures; production sweep and ordering remain linear and insertion remains amortized constant-time. The existing test-only front removal is not a runtime path.
- Runtime capture takes an explicit, unique parent-root candidate-ID set and reads only those keyed custody entries. Candidate discovery and the historical/runtime union belong to the next graph/Actor integration, not capture. No disk, cold session, global runtime scan, startup work or polling is introduced.
- RuntimeInventoryCaptureFuture is a named, owned future. It retains the actual SourceVersion and request-local metadata, not the coordinator borrow or a persistent cache. Active controls are asked once per requested Running capture and are polled concurrently. Its Send capability follows the actual progress future rather than being erased through an unconditional local future object.
- Capture returns the unchanged graph::RuntimeInventory values and their actual graph version. Settling, Writing and Publishing are distinct resident finalization states. Publication custody wins over a stale completed terminal. Full published producers remain strict; evicted output becomes an actual ProducerFact with retained counters and absent output/time, never a placeholder terminal.

## Consequences

The next Actor can store RuntimeInventoryCaptureFuture<<R::Control as ChildControl>::ProgressFuture> alongside its other futures without blocking command processing. It must validate the returned source fence and independently read runtime_inventory_target before accepting a reply or an observed Stop. Lifecycle transitions must advance the canonical graph's source version through its existing evidence owner. Release begin is still only a fence; actual drain and release completion remain the Actor's responsibility.

A complete inventory request must supply every candidate ID from the canonical graph/source union. Capture is not a discovery API, and a captured resident value is not a control grant. Old Query/List/Wait responders and inventory Actor commands still require the bounded C2b migration; no compatibility wrappers were added.

The existing previous-completion constructor and timestamp projection received only the mechanical CompletedOrigin migration. Its persisted format and existing known historical IDs are unchanged. Missing historical execution-start provenance does not become a positive no-start claim. No native recovery implementation was added.

## Status

Authored, unexecuted and not landed. Eighteen runtime-owner/map oracles and two model-boundary oracles were added; all are UNRUN. No local build, check, test, formatter, app, probe, network, live-process or session action was performed. No commit, push or GitLab run was performed. Compilation, CI, C2b integration and end-to-end cancellation remain unverified.
