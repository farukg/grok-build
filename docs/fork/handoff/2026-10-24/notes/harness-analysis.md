# Goal verification and tool availability

Read-only source analysis: researcher `01a166b8-22a3-70c2-b2cf-12430fb74648`, inline result, 29 calls. No source change or runtime reproduction was performed.

## Supported observations

`session/goal_evaluator.rs::build_goal_evaluator_request` creates a separate ConversationRequest with empty tools/hosted tools, no tool choice, an explicit model and required JSON schema. Tool-free evaluation is intentional. It uses a goal-evaluator request ID and parent session/conversation metadata IDs.

`session/acp_session_impl/goal.rs::evaluate_goal_round` calls the separate chat-completion evaluator. CandidateComplete proceeds to `verify_goal_candidate` and `run_verification_stage_for_drain`; failures pause. Main-session request construction remains in ChatState `actor/request_builder.rs`, deriving tool definitions from its session state.

The Responses conversion in `xai-grok-sampling-types/src/conversation/responses.rs` uses no previous-response ID and serializes the supplied request tools/schema. This does not demonstrate mutation of the main tool registry. Shared metadata identity is not runtime proof about every backend cache.

Skeptic children are resolved through `goal_classifier.rs::ChannelSpawner::send_one`, `agent/subagent/handle_request.rs::run_shell_child` and the actual harness tool configuration. This is separate from the evaluator's empty tool list. No historical registry dump supports the earlier claim that the implementer actually lost all tools.

The skeptic spawn uses default capability mode while the summarizer explicitly uses ReadOnly. Effective skeptic permissions depend on the selected harness and parent. This is a policy observation, not evidence for tool loss; no unverified blanket restriction was added.

## Remaining operational observation

After the final verified Grok artifact is staged, exercise a fresh Goal through candidate completion and skeptic verification, then a normal main-session tool action. Capture safe role/session identity, effective capability mode and tool-name/kind inventory at the sampler boundary. Keep auth headers and credential values out of proof. This distinguishes intentional evaluator restrictions, skeptic role restrictions and actual main-session tool loss.

The separately proven Gateway malformed-error producer is repaired and live; its canonical execution proof is `missing-model/release-proof.md`. That repair is not historical proof of steering causality or a blanket restart diagnosis.
