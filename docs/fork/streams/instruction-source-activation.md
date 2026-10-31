# Instruction source and session activation — S1

State: additive source package in the canonical shared tree; uncommitted, SOURCE / UNRUN. No current runtime consumer is switched. The parent owns source review and any scoped publication. `docs/fork/STATUS.md` is not changed by this unlanded increment.

Owner: `xai_grok_agent::prompt::instruction_source`, adjacent to existing prompt/template/rule owners. There is no new crate or dependency.

Architectural source truth: [006 — File revisions and per-session instruction activation](/home/fg/projects/org/governance/sigma/context-modules.d/500_work.d/epics.d/product.d/grok-build/006-instruction-source-activation.adr.md).

## Authored capability

- Exact-byte SHA-256 file pins, stable validated source identity, explicit System/Developer/User request policy, canonical file provenance and existing rule scope/parser reuse.
- Ordered complete disk load with typed missing/I/O/manifest/UTF-8/digest/source-shape errors and original reader/parser causes. No fallback to embedded text.
- Explicit Grok-home or configured-root runtime asset resolution; only existing template filenames, no copied prose catalog.
- Immutable serializable active text/role/origin values, restored without touching disk. Source paths are provenance, not live text references.
- Pure old/new comparison with positions, source values, roles and digests; conservative expected source-prefix change; stale-plan rejection. Only explicit activation returns a replacement value.
- Composed runtime projections can retain verified canonical Sigma-atom/product-source input paths and digests. TODO40 content selection/conflict classification remains with its separate researcher; vendor template text is not thereby approved for future delivery.
- Eighteen pure/temp-file tests cover manual activation, role-only changes, incomplete revision failure, ordering, exact bytes/path identity, deletion-safe restore, fresh/old session isolation, missing runtime assets, existing plaintext template input, rule parsing, manifest boundary failures, duplicate identities/files, projection provenance and additions/removals. All are UNRUN.

## Integration boundary

A manifest is the explicit content-revision boundary. Editing a pinned file without updating its pin produces an explicit load error, never a partial activation. The future editor/composer publishes new pins with changed files; calculating a pin through `InstructionDigest::of_bytes` is pure and does not activate anything. No rebuild or additional confirmation is required by the model.

S2 consumes `load_instruction_manifest` or `load_instruction_snapshot`, `InstructionSnapshot::start` for a fresh session, and `prepare_instruction_activation` / `InstructionActivationPlan::activate` for one existing session. Its durable/session owner must persist and install that value transactionally; session affinity and durable commit do not exist in this value-only S1 package. Session restore/compaction and source/request/transcript views must read the held snapshot.

The old discovery API returns already-parsed `AgentConfigFile::content` and does not retain its named-file/rules-directory distinction. The S1 adapter therefore requires the original discovery format plus the raw-byte digest explicitly; callers must not hash stripped content or invent a second path classifier. A bounded S2 discovery-owner change must expose that original classification/raw pin at construction. S1 adds no scanner and changes no current discovery behavior.

Packaging must install the runtime files and matching manifest at `RuntimeInstructionAssets::in_grok_home(resolved_home)` or supply an explicit root, and make declared projection inputs readable for fresh loads. Existing `templates/*.md` are reused as data, pending TODO40's canonical atom mapping. Existing encrypted generated artifacts and their consumers remain untouched until the owning replacement wave proves retained behavior.

Actual provider conversion, unsupported-role errors, selected-session commands, durable storage/receipts, request-body capture, Pager and automatic/global Skills/MCP activation repairs remain outside S1. No Developer protocol support, measured cost, cache reuse, installed asset or runtime acceptance is claimed.
