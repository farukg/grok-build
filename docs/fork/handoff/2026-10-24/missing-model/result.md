# Missing model after steering — read-only diagnosis

Source: researcher 01a164d0-2b30-7f00-aa3b-ab1a4484482e, inline final result (read-only role cannot write files). No implementation, test execution or restart was performed. A decoder/wire mismatch is evidenced; the malformed provider frame and steering causality remain unproved.

## Observed failure

Parent session: 01a15e70-cc42-70d2-8b7b-a20615d0abf4.
Child: 01a164a2-3412-7770-9e4a-f6a4150b2d1f.

- `/home/fg/.grok/sessions/%2Fhome%2Ffg%2Fprojects/01a15e70-cc42-70d2-8b7b-a20615d0abf4/updates.jsonl:5797`: subagent_finished, failed, serialization error missing field model, 31 model calls and 31 tool calls. Usage says gpt-6.1-sol; that does not establish the missing wire frame's model.
- `/home/fg/.grok/sessions/%2Fhome%2Ffg%2Fprojects%2Forg%2Fforks%2Fgrok-build/01a164a2-3412-7770-9e4a-f6a4150b2d1f/updates.jsonl:122–124`: failed retry_state, serialization error, terminal error.
- Parent updates line5990 contains the user's steering report. This alone does not prove this child is the user's intended affected attempt.

## Exact decoder contract

Root `/home/fg/projects/org/forks/grok-build/`:
- Pager `crates/codegen/xai-grok-pager/src/app/error_display.rs:305–307` maps Serialization to the visible "Couldn't read the response" text.
- Sampling types `crates/codegen/xai-grok-sampling-types/src/error.rs:129–152` wraps serde_json::Error; sampler events.rs270 classifies it. Shell session/acp_session_impl/sampler_turn.rs86–112 treats it as nontransient.
- Sampling types lib.rs53 reexports async_openai Responses types. Cargo pins revision95b52ebdedf42143083cf3d6f0e0be7c84e9c808.
- Readable pinned source `/home/fg/.cargo/git/checkouts/async-openai-80cbf7d6f25979e0/95b52eb/async-openai/src/types/responses/response.rs:2413–2484`: Response.model is required String at 2473, created_at required at 2429; id/object/output also required.
- Pinned `types/responses/stream.rs:159–185`: created/in_progress/completed/failed/incomplete embed Response and sequence_number.
- Sampler `src/client.rs:101–198` decodes known SSE events; malformed known payloads remain errors. Lines1456–1488 decode successful nonstream bodies as Response. Lines1680–1720 handle SSE/error entry.
- Sampler `src/stream/responses.rs:431–450,594–620` assembles completed/incomplete output only AFTER successful wire decoding. Lite output reconstruction cannot repair a decode-time missing model.
- Shell `session/acp_session_impl/parent_message.rs:404–490` handles safe-point steering, persistence and Child gateway projection; it does not construct provider Response.model.

## Gateway boundary and source liveness

Root `/home/fg/projects/org/platform/sigma-crates/infra/gateway/`:
- `src/server/mod.rs:1312–1452` selects native live/JSON/stream or Chat→Responses paths.
- `src/routing/opaque_reasoning/responses_terminal.rs:366–369` internal ResponsesCompleted.model_name is Option<String>; lines1634–1637 read raw `model`.
- `src/server/sse.rs:375–414` stages native/portable terminal projections and supplies created_at; it does not prove model normalization. Internal optional metadata alone is not proof of the actual malformed wire frame.
- BLD's successful Orchestrator request bld-1792692612571-284e68bd9ad44511abd5e6247948055a reports responses_terminal.rs changed concurrently as unrelated input. This owner is externally active. Read-only until ownership is coordinated.

## Versions and existing fixes

Research observed PATH `/home/fg/.cargo/bin/grok`, plus /proc/1504191/exe and /proc/1503863/exe, selecting `/home/fg/.local/state/sigma-binaries/grok/releases/20261022-163830-xai-grok-pager-49b39f4d.bin`. The filename is not a verified commit. Those processes were not conclusively tied to the affected parent. Earlier22fd28fb selection is historical.

- 0ec2b011 tolerates unknown event TYPES, not malformed known events.
- b60e762b negotiates capabilities, not model provenance.
- 22fd28fbacea0619335f6db6d4d4eae529fe53cb assembles output_item.done for empty Lite output.
- b82768d4ad9095ca9fec77aade9a1c4c8ad440b0 preserves committed Lite items on recovery.

No cited fix proves the missing-field case is resolved. Restart sufficiency is unverified.

## Required next work

1. Coordinate the active Gateway terminal owner and capture the affected redacted response event/body, response ID, endpoint/event type, and available model provenance. Exclude authorization headers and unrelated message content.
2. Compare malformed known event against the actual public wire contract. Full Response remains strict. If a native Lite contract legitimately omits terminal fields, classify it explicitly and preserve actual model from verified stream provenance with matching response ID; do not invent a model or use an unverified request alias.
3. Fix the responsible producer/adapter after causal evidence. Preserve cause for malformed error bodies, absent provenance, failed/incomplete events and EOF.
4. Commit regressions for full response missing model, terminal with prior same-ID model, different provider/request model, missing provenance, failed/incomplete, EOF/double terminal/post-terminal errors, and safe-point steering.

No credential, environment or private-key contents were inspected or copied into this report.
