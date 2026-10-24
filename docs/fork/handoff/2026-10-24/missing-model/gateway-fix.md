# Gateway Responses envelope repair

## Delivery status

The source repair and ten pinned-consumer regressions are authored. Compilation, formatting, dependency resolution, test execution and live activation remain unverified and belong to Lead. The supplied green prerequisite is BLD request `bld-1792690877145-8241b8748d16476991bfd0378531f4d6`, with payload `cargo-quick test -p sigma-gateway --locked`. No new baseline gate is required.

## Proven root defect

`NativeResponsesSequence::terminal_error` constructed the known `response.failed` event without `response.model` or `response.output`. Grok's pinned async-openai revision `95b52ebdedf42143083cf3d6f0e0be7c84e9c808` requires both fields. Inspection of its actual `Response` establishes the complete mandatory top-level contract: `id: String`, `object: String`, `created_at: u64`, `status: Status`, `model: String` and `output: Vec<OutputItem>`. Lifecycle events additionally require `sequence_number: u64`. `ErrorObject` requires string `code` and `message`; its code is not an enum. The separate `error` event requires `sequence_number` and `message`, with optional `code` and `param`.

This independently proves an invalid Gateway producer. It does not establish that this producer generated either historical child's missing-model frame, that steering caused the transport failure, or that restarting any process repairs that history.

## Domain ownership and remediation

The server envelope owner is `server/responses_envelope.rs`. It carries distinct selected-attempt and response-ID-bound provider model provenance, typed envelope failures and typed invalid-field identities. Provider model returned for the same response wins. Otherwise the actual selected attempt model is available, not the caller's API alias or billing model. Source provenance is `proxy.rs`: `RouteDecision.selected_model` is populated from `candidate.model_id` at the successful routing decision, and its field documentation distinguishes it from billing identity.

The shared failure serializer now supplies every required Response field, including an empty output array for a synthetic failure. If neither legitimate model provenance exists, it emits a valid typed `error` event preserving the failure cause rather than inventing an empty, unknown or fake model.

Known native lifecycle envelopes are completed only where evidence permits it. Missing model uses legitimate provenance; missing object and creation time use Gateway-owned envelope values. Missing output is admitted only for failed or pre-generation lifecycle states, not completed or incomplete success payloads. Wrong types, missing response identity/status and unavailable model provenance become explicit client-decodable failures. Valid native originals remain byte-exact. Native error events use the existing typed provider-failure decoder to populate their actual public error fields and preserve their cause.

The existing last-forwarded sequence owner still controls synthetic continuation, including checked exhaustion. Missing lifecycle/error sequence numbers use that continuation. Existing numbered valid provider frames are not renumbered. Portable failure projection now retains model and creation time, emits output, and the canonical wire renderer retains the original sequence when projection omits it.

Peer producers are repaired in the same envelope scope: translated Chat preludes and terminals, buffered native Responses, buffered Chat-to-Responses and JSON-to-SSE preludes. The buffered Chat converter no longer invents `upstream-did-not-report-model`; the shipped handler supplies selected-attempt provenance or returns a protocol error. The existing CG full-response producer now uses `created_at` and the mandatory Responses usage field names/details rather than Chat usage fields.

Terminal staging remains transactional. A staged terminal is not a client-visible completed response until clean EOF. Existing upstream-read, partial-event and semantic-idle invalidation tests remain applicable. A clean accepted complete emits exactly its terminal, with no appended synthetic failure. No activity classification, absolute deadline, total cap or terminal transaction acceptance policy was relaxed.

## Exact changed paths

Paths are relative to `/home/fg/projects/org/platform/sigma-crates`.

- `infra/gateway/Cargo.toml`: dev-only pinned async-openai client dependency, with the real Responses API feature.
- `Cargo.lock`: only this unit's SDK, macro and backoff records, Gateway dependency edge and secrecy serde feature edge were added. Existing foreign lock changes were preserved.
- `infra/gateway/src/server/responses_envelope.rs`: new canonical required-envelope/provenance owner.
- `infra/gateway/src/server/mod.rs`: module registration and native/buffered envelope boundaries.
- `infra/gateway/src/server/sse.rs`: synthetic failure context, known native envelope handling, native error serialization and translated prelude/terminal fields.
- `infra/gateway/src/server/responses_chat.rs`: removal of fabricated model fallback.
- `infra/gateway/src/server/sse_anthropic_to_chat.rs`: JSON-to-Responses SSE envelope/prelude handling.
- `infra/gateway/src/server/sse_chat_to_responses.rs`: selected-attempt provenance for translated failures before any upstream model.
- `infra/gateway/src/server/cg.rs`: full-response timestamp and usage wire fields.
- `infra/gateway/src/routing/opaque_reasoning/responses_terminal.rs`: only portable terminal envelope retention and canonical projected sequence retention for this unit. Senku owns the independent event-parser/activity hunks.
- `infra/gateway/src/server/sse/tests.rs`: pinned SDK failure decoding, new child test module, explicit model provenance in direct serializer setup, and complete mandatory envelope fields in native byte-preservation fixtures. The fourteen semantic-idle regressions and their behavioral assertions are preserved.
- `infra/gateway/src/server/sse/tests/wire_contract.rs`: new pinned client/wire regressions.
- `infra/gateway/src/server/tests.rs`: explicit selected-model setup for translated state fixtures and complete model provenance in buffered/native envelope fixtures. Existing assertions are preserved.

No files in codex-rs, Orchestrator, Pager or the separate Grok checkout were edited.

## Regression names

Module: `server::sse::tests::wire_contract` in the sigma-gateway binary test target.

1. `pinned_client_decodes_projected_terminals_with_provider_envelope` covers all three continuation modes and completed/incomplete/failed outcomes.
2. `pinned_response_decodes_full_envelope_without_fake_model_defaults` decodes the full owner-normalized body as the actual pinned Response and rejects different-response provenance reuse.
3. `pinned_client_decodes_upstream_error_before_first_response` covers the native provider error event, preserving its cause.
4. `pinned_client_decodes_failed_terminal_without_provider_output` closes missing output on a failed terminal without inventing success.
5. `pinned_client_decodes_native_failures_before_first_event` covers clean EOF, upstream read failure and semantic idle using selected-attempt provenance.
6. `pinned_client_prefers_same_response_provider_model_on_transport_failure` covers those transport endings after response.created, with distinct API-requested, selected-attempt and provider model identities, byte preservation and sequence 73 to 74.
7. `pinned_client_decodes_native_terminal_model_completion` covers missing terminal model after a same-response created event, retaining terminal sequence 91.
8. `pinned_client_preserves_valid_native_terminal_bytes_without_appended_failure` covers exact completed/incomplete/failed native bytes and one terminal.
9. `pinned_client_decodes_explicit_error_without_model_provenance` verifies a valid explicit protocol error instead of a fabricated model.
10. `pinned_client_rejects_malformed_native_context_as_decodable_failure` covers invalid model, output, creation time, object and status fields.

These regressions drive the shipped `stream_relay_responses_sse` path. Its emitted bytes are then served over a loopback HTTP SSE endpoint and consumed by the real pinned `Client::responses().create_stream`, not a locally weakened lookalike DTO. The existing shared failure assertion also deserializes with the actual pinned `ResponseStreamEvent`, extending that wire oracle across the preserved sequence and semantic-idle regressions.

## Lead compiler and test commands

Execute through Lead's SIG/BLD wrapper, never bare cargo. Payloads:

- `cargo-quick test -p sigma-gateway --locked --bin sigma-gateway pinned_client_`
- `cargo-quick test -p sigma-gateway --locked --bin sigma-gateway pinned_response_`
- `cargo-quick test -p sigma-gateway --locked --bin sigma-gateway native_semantic_idle_`
- `cargo-quick test -p sigma-gateway --locked`

Lead should apply the maintained pinned formatter after Senku's parser/activity join and this envelope join are both complete. The dependency lock additions were authored from the actual pinned SDK manifest and existing compatible workspace versions, without running a resolver. Lead must establish that the first locked build accepts that graph; a Cargo-requested canonical lock adjustment is part of the scoped dependency integration, not a broad baseline blocker.

## Observations

Lead's redacted current live schema evidence is `/tmp/grok-goal-4f7553a1dbe6/implementer/missing-model/live-schema-canary.log`. Its three successful text/tool/round-trip requests report complete created/in-progress/completed envelopes with model present. It supports preserving the current valid native success bytes, which this repair does. It does not exercise this synthetic failure repair and does not reproduce steering or either historical serialization failure.

The earlier diagnosis correctly identified the strict consumer boundary but did not prove a historical malformed provider frame. The source defect fixed here needs no such frame to be a real invalid producer.

## Shared join and unverified limits

Observed HEAD: `ef25d2686398e343e0144a420c8650ca76f3f8af`.

Shared join manifest: `/tmp/grok-goal-4f7553a1dbe6/implementer/missing-model/gateway-fix-join.sha256`.

Shared join hash, SHA-256 of that manifest: `6b2e42f07a4d9fd1a8ad3569392a81a3c0b2733880d66f5f87c0ee95c612f957`.

The manifest is a source snapshot, not a commit or a green build. Senku's separately active parser/event-test hunks can advance shared-file hashes after this snapshot. This unit changes no parser activity matches and does not overwrite his work. His custom-tool-input activity join and the joint SIG acceptance remain Lead coordination responsibilities.

The implementation has no claimed post-edit compiler, formatter, test or live-activation result. No historical child frame, transport causality, steering causality or restart sufficiency is proven. The loopback client regressions validate the shipped relay's output contract; they are not an authenticated end-to-end provider-routing or real steering reproduction.
