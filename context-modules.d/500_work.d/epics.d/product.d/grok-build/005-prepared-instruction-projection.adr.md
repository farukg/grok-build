# Prepared instruction and capability projection

## Context

Final local provider bytes and conversation source membership already exist, but membership alone cannot associate individual provider fragments with their originating sources. Provider conversion changes cardinality, roles and content representation; Responses patches hosted tools after typed conversion, while Messages moves system content into a separate cache-bearing field. The pre-edit recoverable baseline is `a82c8efcedcca938996f50e8061fcc5ccb35a3ca`. Existing uncommitted work is not replaced.

## Decision

The existing emitting converters produce a `ProviderInstructionPlan` alongside their unchanged provider request. References carry the actual emitted field location and declared conversation occurrence/category/content evidence. User instruction parts are selected through the existing named-reason/section classifier, not text or history position. The emitting content branch supplies its representation; observation does not reclassify provider shapes.

At the existing Prepared boundary, the sampler resolves those locations against final serializer bytes, reads actual wire roles and captures final tool definitions, tool choice and structured-output fields. Provider-owned capabilities use request-bound authority, never invented conversation occurrences. `LocalRequestBody` remains the only raw byte owner and carries reference-only metadata bound to request ID, attempt, backend and original body digest. Existing Prepared event fields and error propagation remain unchanged.

The publication API returns only selected values through the existing secret/path scrubber. Exact JSON values are distinguished from original byte slices and redacted values. Missing conversion evidence, missing source identity, malformed fields and unavailable bodies remain typed. The content digest excludes source IDs, request coordinates, locations and instruction cache breakpoints; it is an equality aid, not a logical occurrence ID. Every preparation remains observable, including A→B→A and retries.

## Consequences

The existing session capture/publication owner can retain selected projections while evidence is available, compare consecutive effective states and version real transitions. S1 introduces no session store, deduplication ledger, UI, provider-role configuration or instruction reload. Declared source, active final values and final wire role remain separate. Disk files are never consulted by observation, so discovery cannot silently activate different instructions in running sessions.

Metadata work is proportional to instruction references and final capability entries. Final JSON is parsed at preparation and on requested publication; those temporary values are not retained as another whole-request store. No startup I/O, polling or frame work is added. Runtime performance and final integration remain unverified.

## Status

Authored, UNRUN. Provider golden/oracle tests are present; no build, check, test, formatter, network action, commit or push was executed. Durable session transition ownership and normal-chat rendering belong to separate workers.
