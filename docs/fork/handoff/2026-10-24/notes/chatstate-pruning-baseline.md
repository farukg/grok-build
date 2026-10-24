# ChatState pruning baseline — static diagnosis only

No build/check/test/formatter ran. No pruning source or assertion was edited.

## Exact mismatch

- `crates/codegen/xai-chat-state/src/actor/tests.rs:4259–4285`, `apply_turn_request_pruning_is_noop_under_half_window`, creates five turns with 8,000-byte tool results, records provider usage 4,000 against a 10,000-token window, and independently expects the oldest tool result to remain 8,000 bytes.
- `actor/mutations.rs:460–465`, `record_token_usage`, resets the post-response delta, calculates `estimate_at_last_response` from the entire retained conversation and separately stores `TokenCount::Fresh { tokens: total_tokens }`.
- `actor/request_builder.rs:123–131` gates pruning on `estimate_at_last_response + estimated_tokens_since_model`, **not** the recorded provider total. Five 8,000-byte tool results alone estimate 10,000 tokens. Thus this path exceeds the 5,000 threshold regardless of the recorded 4,000 usage.
- `actor/request_builder.rs:140–145` explicitly documents the competing design: retained-history estimate is intentional because provider totals describe an already-pruned request and using them can alternate pruning on and off. The observed 3,019 bytes matches a soft trim rather than a missing tool result.

This is a policy/oracle inconsistency in the published baseline, not evidence that completion changes caused token-accounting corruption. It is not yet a proven unintended production defect. Simply replacing line 128 with the provider total would satisfy this one test while violating the explicitly documented anti-oscillation policy. Changing the expected result to 3,019 would abandon the current independent no-prune oracle and is not authorized here.

## Owning surface for a separate worker

`xai-chat-state/src/actor/{request_builder.rs,mutations.rs,state.rs,tests.rs}` and the request/compaction token-estimation regressions. Context projection is concurrently owned; coordinate before changing the accounting policy. No completion-admission file needs to change to resolve it.

## Required resolution

Establish which existing contract wins: (A) trusted fresh provider occupancy controls the half-window gate, with separate stable post-pruning/request accounting to prevent oscillation; or (B) retained-history size controls pruning even when provider occupancy is below half, in which case this test is genuinely obsolete and needs an explicitly approved replacement behavior oracle, not an unexplained assertion adjustment. Reproduce through the real request builder and two successive model responses before choosing an exact root-fix diff. No verified corrective hunk is supplied because neither policy is established by the reported single failing test.
