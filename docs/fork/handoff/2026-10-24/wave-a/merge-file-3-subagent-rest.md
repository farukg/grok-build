# Wave A merge report — file 3

Status: done. Resolved all six conflict regions in `crates/codegen/xai-grok-shell/src/agent/subagent/tests/rest.rs`.

## Per-region decisions

- Completion disposition fixtures: retained the runtime lane's `foreground_delivered: true` fixture shape in both cases, while preserving the canonical producer-admission assertions and persisted completion receipt checks.
- Producer-ack snapshots: the first fixture captures `committed_data` after acknowledgement, after verifying the actual producer event is `Completed`; the separate pre-commit case retains the Running metadata assertion before admission.
- Replay bindings: after the second publication, the parsed metadata is named `repeated` and compared against the committed terminal metadata; the follow-up no-commit test keeps its own terminal result assertions.

## Phase oracle

Before `prepare_completion_admission`, metadata remains Running, has no completion timestamp, and has the snapshot reference; persisted completion history is absent. After producer acknowledgement, metadata is compared as a whole against the expected Completed value including producer timestamp, completion cause (`error: None`), and result metrics where provided. After publication and replay, the serialized metadata bytes remain identical and the parsed terminal value remains unchanged. The fixture proceeds through admission and publication entry points; receipt/history assertions remain present.

## Open questions

None. No build or test verification was performed, per author-only scope.
