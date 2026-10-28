# Native wake crash adoption

## Context

A native wake can durably admit its parent message before the initial publication handshake completes. The receiver's existing JSONL custody owns payload, projection and terminal state. Reconstructing an ordinary prompt on startup discards initial result custody and launch configuration. Admission uncertainty without an identity cannot be observed safely.

## Decision

Extend ParentMessageAdmission atomically with typed initial launch custody. Keep message text exclusively in the existing admission/execution payload owner. Retain schema, trace, parent attribution, management-recipe binding, original receiver generation, attempt, prior completion boundary and publication context. Store only stable artifact destination/principal bindings and references, never credentials or runtime clients.

Role/persona text remains solely at ChildManagementRecipe. The existing native claim carries that typed recipe in memory to the same receiver-owned admission transaction, which verifies its binding and durably synchronizes its canonical file before acknowledging the admission. Native preparation no longer rewrites this file before admission. A missing file can be re-resolved only from authorized broker data with the exact original binding; changed data is not overwritten. A fresh initial delivery cannot overtake an older admitted, unprojected initial intent and destroy its previous-result custody.

Startup observes parent custody without replay. An explicit same-audience, authenticated recovery request may reauthorize an unprojected initial delivery through the existing coordinator, receiver lease, task reservation, results receiver, deferred initial publication and start gate. The receiver revalidates the exact durable record. Projected, interrupted and terminal records are observations, not fresh attempts. Durable authority cannot be revoked into a definite rejection by an unsuccessful adopter.

Previous completion metadata retains the existing file-backed result owner; neither its prompt nor output payload is copied into admission custody. The coordinator distinguishes restoration of that prior completion from adoption of an already-published attempt. A payload-free publication checkpoint retains model, reasoning, trace counter and stable agent references; inline definitions are represented by a canonical binding digest, never copied credential-bearing configuration. A committed WakeStart prefix with the old publication still present is rearmed inside the same receiver-owned admission transaction. An already-published unprojected attempt instead confirms its existing prefix, reacquires the real results/reservation/start-gate custody, and does not republish its start or increment trace state again.

Artifact recovery requires explicit re-resolution of the same stable destination and authority: the existing wire-valid proxy login principal, SDK-decoded service-account identity/token audience, or named absolute S3 credential-file reference. ADC, inline S3 and deployment-key capabilities without a named stable authority resolver remain correlated uncertainty; matching current defaults never authorizes them. A sealed in-memory grant carries the classified binding and the existing artifact context; credentials and clients are never part of serialized custody. Legacy and live custody without initial publication ownership is not reconstructed as an ordinary prompt.

Current AdmissionUncertain outcomes require recoveryRef through tool and Shell outputs. The pager preserves the exact reference in its existing unconfirmed presentation. Older stored tool outputs that omitted correlation decode as HistoricalUnavailable: they remain unconfirmed history, cannot authorize recovery, and are never the result of converting a current native outcome. No empty reference is synthesized. Observe/Adopt accepts an exact reference, not message text. Startup delivery observations add that same derived reference to the existing notification.

The existing owning-parent scope is established by both persisted parent bindings and the live authenticated session; it may manage either original message lane without changing sender/source. A granted child additionally requires its current verified holder, attempt/generation and existing root lineage, and may reauthorize only its own Agent sends.

## Consequences

Failed or unavailable recovery preserves the previous completed attempt and artifacts. No new prompt identity, payload store, universal controller, polling or startup I/O is introduced. Explicit recovery reads the existing custody/history owner to distinguish a committed projection from a queued record.

The active MvpAgent dispatcher must wire the typed Shell recovery handler using its private coordinator sender. Protected caller tests must consume the new identity-bearing outcome. Local execution is prohibited; native integration and fault oracles are source-only until authorized GitLab verification.

## Status

Authored, not verified. The shared dispatcher/caller hook and authorized GitLab verification remain integration gates.
