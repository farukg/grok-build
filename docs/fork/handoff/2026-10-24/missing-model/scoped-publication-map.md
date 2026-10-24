# Scoped Gateway publication map

## Decision

A bounded publication is feasible without carrying the 14-commit shared branch, but it is **not** a clean whole-commit cherry-pick. The independently useful scope has two ordered source slices:

1. GPT-6/GPT-6.1 Responses Lite runtime compatibility (`c3a0…` plus selected consumer hunks from `f64…`).
2. Responses envelope wire repair and Senku's semantic-idle/custom-tool-input correction (selected Gateway hunks from `c618…`).

Use a clean publication branch based on local `origin/main` **`be6c8ebc7ecf3a00ee16bff58d0b611dbd579986`**. That ref was read locally; no fetch was performed. Do not cherry-pick the checkpoint chain or copy the live working tree wholesale. Preserve separate ownership for the model/probe/generator consumers and the shared Senku Gateway hunks. The closure below is a static path/hunk map—not proof a filtered patch applies cleanly or passes tests; no branch was created.

This only covers the runtime-compatibility and wire/idle scope. It does not satisfy the larger `gw-codex-lite` WebSocket/Responses/Summary/Conversation/Cache/Compaction acceptance. The current Gateway README explicitly says WebSocket transport remains unimplemented.

## Exact local history

The observed local refs were `origin/main = be6c8ebc7ecf3a00ee16bff58d0b611dbd579986` and `main = 4e8ebedee3fbfafa1b6dada73cd3c107a62bb21d` (14 commits ahead). The ancestry and commit dates are:

| Full SHA | Commit date (+02:00) | Subject / scope relevance |
|---|---|---|
| `96137308c058e3fe4273bc8ea31c1a2c38ea2926` | 2026-10-22 11:56:39 | User checkpoint; build reported successful |
| `936897b6b03b5a0902e9e33b49c4fdfa082d9834` | 2026-10-22 12:35:38 | Mixed provider-capture checkpoint; adds Gateway shadow-capture wiring |
| `4c1360f205f12c585659da6bdd6a70838213df68` | 2026-10-22 12:57:45 | Provider-capture evidence/release work; changes Gateway shadow capture |
| `c3a0fc12334439f1f370234ee803b9e00ba5dee4` | 2026-10-22 13:13:50 | Gateway Responses Lite tool-transport header; one Gateway source file |
| `03fcfeb9fbd57fb87a1792df9f60fbf27388ab92` | 2026-10-22 13:21:21 | Capture-evidence retention checkpoint; no Gateway source path in its file set |
| `f64f1ff0dc90f13fb265094ef6d29014dcc91c75` | 2026-10-22 16:02:28 | GPT-6 native Responses Lite runtime compatibility; mixed Gateway/model/probe/generator commit |
| `d9ed9a6b13c47786cb826f314bb648ebcd602953` | 2026-10-22 16:21:04 | Observability archived-request changes |
| `0680593838b1ee33c53eb7312afc2b8fb7c5316d` | 2026-10-22 17:01:35 | Provider-capture metrics/outcome sampling |
| `328feb3c97d2b24ad0f1219e72a88924fd8abde7` | 2026-10-22 20:23:13 | Durable capture/task-evidence consumers; includes Gateway capture plumbing |
| `ef25d2686398e343e0144a420c8650ca76f3f8af` | 2026-10-22 20:39:06 | Capture-evidence contract checkpoint |
| `c618b3dfbc6b03e52ba584d6e0cf0cbfbdbf724c` | 2026-10-23 00:13:28 | 79-path mixed user checkpoint containing the Gateway wire repair and semantic-idle hunks, plus unrelated work |
| `cd9a0ad4ef064e7b9b11ca656a97481e3de30c0e` | 2026-10-23 04:58:52 | Physical-copy policy/drift work; no Gateway path |
| `37621b157df57d1334c9be177cf16320fbe1e898` | 2026-10-23 09:15:44 | Legacy capture preservation; no Gateway path |
| `4e8ebedee3fbfafa1b6dada73cd3c107a62bb21d` | 2026-10-23 10:39:31 | Legacy preparation probes; no Gateway path |

`c618…` is a descendant of `ef25…`, not a publication-ready commit: its stat is 79 files, 8,081 insertions and 1,122 deletions. It includes Orchestrator, secrets/scanner, capture-evidence and cargo-localbuild work as well as Gateway. The three later commits contain capture changes and lockfile changes only, not Gateway source. None of `936…`, `4c136…`, `03fc…`, `d9ed…`, `068…`, `328…`, `ef25…`, `c618…`, `cd9a…`, `37621…` or `4e8…` should be taken wholesale for this Gateway publication.

## Candidate dependency closure

### A. GPT-6 / GPT-6.1 Responses Lite runtime compatibility

`c3a0fc12334439f1f370234ee803b9e00ba5dee4` changes only `infra/gateway/src/proxy.rs`. It sends the Codex Responses Lite negotiation header when the selected candidate action moves tools to `additional_tools`; its helper and test can be selected independently. The `ResponsesToolTransportAction` interface already exists on the observed `origin/main`, so the earlier `936…`/`4c136…` capture ancestry is not required for this hunk.

`f64f1ff0dc90f13fb265094ef6d29014dcc91c75` is mixed-owner and must be filtered, not cherry-picked whole. Include these source/consumer paths and their runtimecompat hunks:

- Gateway request and routing: `infra/gateway/src/openai_body.rs`, `infra/gateway/src/openai_body/tests.rs`, `infra/gateway/src/proxy.rs`, `infra/gateway/src/routing/candidate_metadata.rs`, `infra/gateway/src/routing/candidate_metadata/tests.rs`.
- Gateway Responses handling/tests: `infra/gateway/src/server/mod.rs`, `infra/gateway/src/server/native_codex_tests.rs` (new), `infra/gateway/src/server/responses_tools.rs`, `infra/gateway/src/server/sse.rs`, `infra/gateway/src/server/sse/tests.rs`, `infra/gateway/src/server/tests.rs`.
- Provider/model consumers: `shared/provider/src/model_catalog/codex.rs`, `shared/models/src/project.rs`, `infra/probe/src/model_discovery.rs`, `infra/probe/src/model_discovery/tests.rs`, `gen/agent-config/Cargo.toml`, `gen/agent-config/src/render/codex_catalog.rs`.
- Current-truth docs: `infra/gateway/README.md`. Its `origin/main` text still says `src/server.rs` and omits the native GPT-6/Responses Lite contract. The `f64…` update corrects the source path and documents Lite request controls, native opaque event handling, the output-item consumer contract, and that WebSocket remains unsupported; include that existing README-owner update with this runtimecompat slice.

The consumer chain is concrete: `CodexDefaultServiceTier` adds an `Available(CodexServiceTierId)` case; probe parses a named default tier; the model catalog/generator preserve it; `sigma_models` selects native OpenAI Responses; Gateway then applies the catalog-derived tool, instruction, reasoning, context-management and service-tier policy. All matching consumers of `CodexDefaultServiceTier` were bounded to the provider, probe and generator sites above.

Exclude `infra/probe/src/providers/anthropic.rs` from `f64…`: its Claude Haiku default change is unrelated to the GPT-6 contract. Do not import unrelated hunks from `README` or model-discovery formatting as silent payload; carry the checked README correction and only the parser/type hunks required by this contract. The commit message records Gateway/model/probe/generator verification and live tool continuations for GPT-6 Astra, Sol, Luna and GPT-6.1 Sol, but no matching CI run IDs were present in the read evidence; treat it as author/commit evidence, not new-branch verification.

### B. Responses wire repair and semantic-idle closure

`c618b3dfbc6b03e52ba584d6e0cf0cbfbdbf724c` contains both Gateway Wirefix work and Senku's semantic-idle work. There is no separate clean commit for these source hunks. Select these exact Gateway paths from the checkpoint delta, preserving the existing same-file owners:

- `infra/gateway/Cargo.toml`: add only the pinned **dev** client `async-openai` at rev `95b52ebdedf42143083cf3d6f0e0be7c84e9c808`, feature `responses`. Do not pull the `sigma_provider_observability` dependency from the capture-consumer chain unless a selected regression demonstrates it is required.
- `infra/gateway/src/server/responses_envelope.rs` (new), `infra/gateway/src/server/mod.rs`, `infra/gateway/src/server/cg.rs`, `infra/gateway/src/server/responses_chat.rs`, `infra/gateway/src/server/sse.rs`, `infra/gateway/src/server/sse/tests.rs`, `infra/gateway/src/server/sse/tests/wire_contract.rs` (new), `infra/gateway/src/server/sse_anthropic_to_chat.rs`, `infra/gateway/src/server/sse_chat_to_responses.rs`, `infra/gateway/src/server/tests.rs`.
- `infra/gateway/src/routing/opaque_reasoning/responses_terminal.rs` and `infra/gateway/src/routing/opaque_reasoning/responses_terminal/tests.rs` (new).

Keep the complete authored wire contract: legit selected-attempt/same-response provider-model provenance, mandatory Response fields, typed failure when provenance is absent, portable output/sequence retention, exact native passthrough when valid, correct CG usage fields, and one terminal after clean EOF. Preserve the 10 pinned-client/wire regressions in `gateway-fix.md`.

Also keep Senku's semantic activity/idle hunk set in those shared files: `ResponsesSemanticEvent::has_semantic_activity`, typed decoding of `response.custom_tool_call_input.delta`, acceptance of empty strings as non-activity without accepting malformed/missing text, the relay's semantic-only idle-deadline refresh, and the accompanying event/deadline regressions. In particular, `server/sse.rs`, `responses_terminal.rs`, and their tests are co-owned; do not copy an old whole-file replacement over either contributor. The observed `origin/main` lacks `has_semantic_activity` and the custom-tool-input parser, so omitting this set would not be a complete publication of the proven joined hotfix.

### Lockfile boundary

Do not copy `Cargo.lock` from `c618…` or the current Main worktree. The checkpoint lock diff contains the intended `async-openai` 0.33.1/macros 0.1.1 pinned git records, `backoff` 0.4.0, the Gateway edge and secrecy serde feature, **plus unrelated** `sigma_output_render` 0.1.4→0.1.5 and `tree-sitter-python` records and other workspace resolution changes. The post-checkpoint live lock adds generator/config/observability package version updates and `windows-sys`/`socket2` changes. Resolve the exact selected closure on the clean branch through the authorized SIG build/update path; review the lock diff against the branch base and retain only resolver-required records. A locked test must then prove the resulting lock graph is stable. No hand-selected lock surgery is evidenced.

## Explicit exclusions / active state

- The capture-provenance feature is a separate dependency domain: `infra/gateway/src/shadow_capture.rs`, `infra/gateway/src/usage_ledger.rs`, its `sigma_provider_observability` manifest edge, and matching `936…`/`4c136…`/`328…` hunks are not needed by the described envelope/Responses Lite tests. Do not include them by transitive-looking commit ancestry. If a clean-branch compile proves a real API dependency, stop and request the specific owner join rather than importing the capture series wholesale.
- Do not include any of the three post-`c618` capture-evidence commits. They are not part of the Gateway path closure; their successful legacy-owner tests do not re-verify Gateway.
- Current working tree after `c618…` has formatting changes in eight Gateway Rust files and additional `Cargo.lock` changes. Existing reports say the pinned Gateway formatter passed, but the live artifact is the tested pre-format binary. Do not take current working files blindly; apply the exact source/hunk closure, run the pinned formatter, and reverify the resulting publication SHA.
- Keep current and historical truth distinct: the deployed binary SHA is `e212880479b2a050973fa992138212086ad2960684c19a55801c19df4a5919eb`; release proof says its artifact predates the later formatting. Successful live text/tool/tool-result requests do not test the synthetic failure path or prove historical steering causality.

## Existing proof and what it does not prove

- Source checkpoint: `c618b3dfbc6b03e52ba584d6e0cf0cbfbdbf724c`.
- Full Gateway tests: BLD `bld-1792704581745-6c57286a5cf14413a64b87420f5df417`, success/zero warnings; captured output `missing-model/gateway-tests-success.log` (204 library tests, 589 binary tests, doc tests 0). It includes the semantic-idle regression names and pinned wire tests from this checkpoint.
- Gateway executable build: BLD `bld-1792704839150-5d5479271d454085a45b987e9f44cb2c`, success. Active snapshot `/home/fg/.local/state/sigma-services/sigma-gateway/releases/20261023-000958-sigma-gateway-e2128804.bin`, SHA above.
- Canary and post-promotion evidence: `missing-model/canary-schema-roundtrip.log`, `missing-model/post-promote-schema-roundtrip.log`, `missing-model/live-schema-canary.log`; successful real Sol text/tool/result calls and valid success envelopes. They do not inject production failures.
- Pinned format evidence: `missing-model/gateway-format-apply.log`, `missing-model/gateway-format-check.log`; report says check passed after formatting. It does not prove a post-format build or that the running binary matches formatted source.
- Capture Legacy follow-up builds `bld-1792734089686-b01a696e7329410fa38d203a2c782f2a` and `bld-1792734178313-6cc1138f603c4863bbd82e2354a6d833` verified observability/CLI capture targets, not the Gateway publication closure.
- `f64…` and `c3a0…` commit messages assert runtimecompat tests; the exact source/current integrations and cross-package invocations must still be re-established for the filtered branch.

## Current publication-tooling and registration gates

- Samus reports the exclusive `sig` push-policy tests passed 48/48. Deployable `sig` build request `bld-1792754877441-0d7e62fa42b148df9549c831f08400f6` is currently active with Samus as requester/subscriber. Do not duplicate it; consume its completion event. Until it completes, there is no verified deployable CLI artifact from this request. Its result does not verify a filtered Gateway source branch or its runtime.
- The `sig` CLI build graph directly depends on `sigma_session_orchestrator` and `sigma-headless-adapter`. These are build-time consumer dependencies, not source paths required by the Gateway wire/runtimecompat closure. A CLI build proving those dependencies does not authorize importing their source changes into the Gateway branch.
- Parent/Child binding ownership remains a separate integration/activation gate: Architect `01a1682d-ea31-7573-8b4b-cbed2694eacf` and E03 Architect `01a16898-59be-7eb2-a617-6e3a4f8f68ae` own their registration/binding results. Their reports/source changes do not overlap the Gateway path closure. Preserve the existing session/owner path; do not create another registration session or treat this gate as a reason to expand the Gateway patch.

## Minimal Lead transaction

1. After the registration/source-owner gates and current Shikamaru join are settled, create a clean topic branch from exact base `be6c8ebc7ecf3a00ee16bff58d0b611dbd579986`; do not rebase or publish the shared `main` chain.
2. Apply `c3a0…`'s one-file Responses Lite header hunk; then apply the selected `f64…` runtimecompat consumer closure above, including the README truth update. Omit the Anthropic provider change and all provider-capture hunks.
3. Apply selected `c618…` wire and Senku semantic-idle hunks from the explicit path list above. Split patches/commits by responsibility where possible; for shared files preserve both exact hunks. Include no unrelated checkpoint files.
4. Resolve the branch-local lockfile via SIG/BLD, then run the repo-pinned formatter. Review the full scoped diff and exact paths before testing.
5. Re-run the exact regression scopes on that branch through `sig build` (registered requester/subscriber and project/context supplied before the Cargo payload):
   - `cargo-quick test -p sigma-gateway --locked --bin sigma-gateway pinned_client_`
   - `cargo-quick test -p sigma-gateway --locked --bin sigma-gateway pinned_response_`
   - `cargo-quick test -p sigma-gateway --locked --bin sigma-gateway native_semantic_idle_`
   - `cargo-quick test -p sigma-gateway --locked`
   - Also test the changed consumer packages `sigma_provider`, `sigma_models`, `sigma-probe` and `agent_config_generator` through SIG/BLD; verify the generator's relevant Codex catalog projection. Do not substitute previous `c618…` results for the filtered branch result.
6. Build the exact branch SHA through SIG/BLD; use the authorized bounded common CLI/service activation path with Shikamaru only after the build and source-identity/review gates. For new activation, verify the exact artifact hash, selected release and active process hash, then run real success and failure-path checks appropriate to the deployment. Do not claim the old live SHA is the new branch's binary.
7. Commit the reviewed scoped closure, run required CI for that exact full commit, and publish the topic through the established trusted GitLab route using ordinary non-force policy. The parent-provided destination note says no additional scanner gate/skip flag for `gitlab.myicecreamlab.com` or its established alias; that does not waive independent ref, CI, AIAG or force-policy gates. Publication alone is not deployment. Do not publish the 14-commit shared Main range.

## Ownership decision for Lead

No decision is required to exclude the capture/secret-scanner/Orchestrator checkpoints; they are outside the tested API closure above. The only shared-source ownership boundary is the Senku semantic-idle parser/activity change in the same `c618…` Gateway files as the Wirefix. Keep both; do not ask to omit Senku's correct fix. If the clean branch requires any `shadow_capture`/`usage_ledger` API hunk to compile or preserve the approved Gateway behavior, report the exact symbol/path and ask its capture owner for a reviewed dependency join before expanding scope.

Report assembled read-only from local refs and existing proof files. No fetch, branch/worktree creation, build, test, commit, push, staging or source edit was performed by this map. The separate Samus-owned `sig` build request above remains active and was not duplicated.
