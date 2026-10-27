# Read-only next-request candidates

## Context

P1 owns occurrence identity, content evidence and the common chat-state builder. Context owns the effective revisioned policy. P2 owns evidence of actual serializer attempts and its storage/publication. A next-request prediction must not activate a session, apply policy, admit inputs, consume queues or masquerade as an actual attempt.

The supplied source baseline is `e3a536f1`. The pre-extraction owner files are recoverable in `/tmp/grok-build-lead-20261027/preview-baseline/source.tar`; `turn.rs` was separately copied there before its pure structured-output constructor was extracted. No Git mutation was used.

Source-derived retained capability inventory:

| Baseline capability | Target owner | Authored verification |
|---|---|---|
| Chat/Responses/Messages DTO conversion and defaults, including Messages token fallback | sampler `request_body.rs` | existing defaults/provider oracles; real HTTP bytes compared with preview |
| Chat streaming wrapper and argument repair | same DTO conversion and `chat_body` | existing independent Chat argument oracle and HTTP bytes |
| Responses hosted tool splice, additional includes, ZDR default, native schema and reasoning discriminator repair | same DTO conversion, defaults and `responses_body` | existing schema/hosted/repair oracles plus additional-include assertion and HTTP bytes |
| Final JSON, capture before compression, actual dispatch lifecycle | `json_body` then existing client transport/capture | unchanged capture/HTTP/compression oracles; preview produces no capture events |
| Streaming-only traceparent propagation; nonstreaming behavior | typed `BodyMode` in shared wrapper construction | retained source branches and existing client tests |
| Context projection, pruning, memory, image budget and ContentEvidence | existing chat-state builder, request-local PreviewWith inputs | actor history/policy/persistence/event invariance and active-tool regression |
| Structured-output function declaration | existing turn owner `structured_output_tool_spec` | same constructor used by the real turn and preview |
| Normal prompt assembly and send-now envelope | existing prompt parser/render/layout and interjection core | real session-handler draft body oracle and read-only attachment/queue assertions |

The old private client defaults methods and inline provider wrapper/body constructions were removed in the same extraction; callers now use the maintained pure owner. No public sampling capability or P1 entry point was removed. Real `build_request(...false...)` remains Sampling, not Preview. No P2 storage/capture/publication implementation was replaced.

## Decision

- `SessionCommand::PreviewRequest` is the local full-input boundary. Its `RequestPreviewRequest` contains session identity, UI generation and `PreviewInput::{Current,Draft}`. Draft delivery is a closed Prompt/Queue/Interjection/SendNow enum. Draft-bearing request types have neither default serialization nor content-bearing Debug.
- `RequestPreview::{CompleteAsOf,Partial,Unavailable}` is disjoint from actual request/attempt evidence. Partial requires a first typed late-bound cause. Snapshot identity consists of session, actual binding incarnation, a content-addressed canonical-history revision, effective policy revision and UI generation. The history revision is BLAKE3 of the actor's atomic canonical-history clone including its existing local identity/evidence fields; it is not a model counter or admission ID.
- `ContextPolicyStore::preview_snapshot` only reads an already-loaded authority and already-inspected/authoritative session choice. Uninitialized authority or unknown legacy inspection is unavailable. Preview holds the authority read guard across its build without invoking admission, setters, migration or the applied-revision cache.
- `ChatStateHandle::preview_request_with_context` atomically returns canonical history, sampling config and the existing builder's result. Additional draft items and policy are request-local. Stable preparation retains deterministic request-local repair. InFlight preparation does not synthesize a cancellation result for an unfinished tool call. Draft token estimates flow through the existing pruning owner. Neither mode normalizes/adopts new source IDs, persists, changes capture offsets or emits image-budget events.
- `SamplerHandle::preview_request` is a read command against its existing configuration. It does not construct a SamplingClient, resolve auth, allocate request/attempt identity, submit, emit capture events or touch the active set. It uses the exact provider wrapper/default/body helpers used by HTTP. Backend/model disagreement is unavailable rather than silently relabeled.
- Current tool schemas use the existing loaded registry and child-safe projection; hosted tools and schema declarations use the real owners and Context gate. The root wait tool and StructuredOutput declaration are not independently rebuilt. Schema validation uses the same compiler with an explicitly refusing resource retriever: invalid or externally unresolved schemas are SchemaNotPrepared, never a network/file fetch. The output budget clamp is read-only. Native body bytes are final local serializer output for the candidate, before lossless content encoding; they are not actual-send evidence.
- Queue entries and pending interjections are cloned from their owners, never drained or promoted. Interjections share the core formatter and sanitizer. The current server-selected prompt mode and queued mode are read without changing plan state. Unresolved mode/template preparation, pending skill/command expansion, file/image work, large-prompt offload, first-turn memory, interrupt envelopes, active output and scheduled injections remain typed metadata outside the candidate payload. Current cwd is a local snapshot field, not a fabricated model item. No template effects or explanatory placeholder items are run/appended.
- Raw local bytes remain inside the existing opaque LocalRequestBody contract. Explicit publication uses its maintained scrubber and ExactLocalJson/Redacted representation. Published snapshot Debug hides its body. Diagnostics and preview metadata never enter model history.
- The safe current-state RPC is `x.ai/session/request_preview` with `RequestPreviewRead { session_id, ui_generation }`. It uses only resident handles and rejects binding replacement. It never loads a cold session or touches P2 storage. It returns `PublishedRequestPreview`, not actual request evidence.

## Consequences

The loaded-session producer is usable locally with a draft and remotely for Current. The UI must compare session/binding/history/policy/UI-generation keys and retain normal history as its default. Readable/raw/shortcut-filtered rendering is a later consumer, not part of this change. Candidate raw must retain its preview label and publication representation.

There is a precise transport prerequisite for remote unsent drafts: the pinned `agent-client-protocol` 0.10.4 `src/rpc.rs` traces full outgoing/incoming lines at lines 175, 189 and 211. A handler cannot undo that pre-handler disclosure. No draft RPC parameter was introduced, no logger flag bypass was added and no registry dependency was patched. The transport owner must provide body-private request/response logging before a later UI sends a draft over generic ACP RPC. Until then the full-input command is local only. RPC parameter parsing uses a fixed error, not serde values containing a draft.

The current producer intentionally returns Partial for unpeeked scheduled preparation and model configuration refresh, and may defer attachment/command work altogether. A pending interrupt envelope has no non-consuming tracker accessor in the allowed owner surface, so it remains explicit late-bound metadata. CompleteAsOf is not a claim that these missing owners have been implemented. Active-turn schema is reused when its actual queued owner remains available; otherwise it is explicitly late bound. Runtime races, backlog and user-triggered serialization cost remain unmeasured. There is no polling, frame work or new startup I/O.

## Status

Authored/provisional and unexecuted. Source tests cover atomic nonmutation, authority newer than applied binding without application, typed failures, draft/attachment/queue boundaries, cold unavailability, opaque Debug, and preview equality with actual HTTP bytes for all three streaming backends including additions/repair. No build, check, test, formatter, Git mutation, push, restart or nested agent was run. Remote draft transport privacy and authorized compiler/runtime acceptance remain open gates.
