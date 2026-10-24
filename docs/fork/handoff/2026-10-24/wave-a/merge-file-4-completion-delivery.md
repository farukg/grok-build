# Wave A merge — completion delivery

Status: resolved.

File: `crates/codegen/xai-grok-shell/src/session/completion_delivery.rs`

## Conflict decisions

1. **Completion reference projection:** Kept the canonical `completion_reference_text(id, source)` contract: it renders the neutralized, wrapped durable-result reminder using the completion ID, without embedding event text. `CompletionPayload::model_text` owns composition of the original text and reference. This preserves canonical attempt-bound reference generation and avoids duplicating the event text in stored payloads.
2. **Publication completion:** Kept canonical admission handling and policy decisions, while using the runtime-compatible payload representation: store `event.text` with `reference: Some(reference.clone())` for Pending and Projected obligations. This leaves the payload's reference structured/serializable, so receipt-driven projection remains exactly once and delivery materializes text through the existing model-text projection. Silent and Settled obligations remain removed; already Pending/Projected obligations remain unchanged.

The runtime typed child wakes and terminal-metadata-after-durable-commit behavior are outside these conflict hunks and remain intact in the assigned file's surrounding API/state representation.

## Open questions

None.

Conflict markers are absent from the resolved file. No build or tests were run, per the author-only task constraints.
