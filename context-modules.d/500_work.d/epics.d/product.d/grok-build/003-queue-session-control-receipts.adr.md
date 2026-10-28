# Queue session-control authority and bounded receipts

## Context

The leader multiplexes clients with full ACP session control. Subscriber/driver routing and clientIdentifier are not principal or ownership proofs. Queue row owner is optimistic concurrency metadata. The existing SessionHandle is the internal session-control proxy.

Queue operation receipts previously retained complete requests without an admission bound. The pager replaced its existing queue mirror with older same-ID snapshots.

## Decision

Mint a non-deserializable QueueCaller from the resolved SessionHandle at ACP ingress. Carry that session-control grant separately from QueueRequest through the actor command and receipt owner. Same-session authorized operators may control and observe each other's queue operations. Request metadata supplies no authority.

Reserve finite receipt count and byte capacity before mutation under the existing queue lock. Retain operation identity, caller grant, edit digest and response, not rejected edit payloads. Capacity refusal has no queue mutation; known operations remain observable and idempotent. No expiry or eviction is introduced. The existing IPC message limit also bounds queue protocol input.

Reconcile same-session versions in AppView's existing queue mirror before replacement. Binding metadata contains no queue payload. Omitted or started rows are removed normally, and a changed session-binding epoch resets version continuity.

## Consequences

A live actor can refuse new operations once its receipt budget is exhausted. Receipts remain actor-local; unknown observations do not claim a start or terminal outcome. There is no new per-client ownership or partial-session ACL.

## Status

Source implementation and regression tests authored; unexecuted. Native-fenced SessionCommand and run_loop caller propagation must be integrated before this transaction is buildable or accepted.
