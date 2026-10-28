# Correlated compaction progress

## Context

Baseline: `0b1c204ebd954fbd58c5c78519a459de6bacf980`.

The shell already supports parallel, snapshot-only prefire pass-1. Pass-2 and full
history replacement run at conversation boundaries. Manual compaction admission
retains the original RPC, predecessor ordering, explicit stop and persistence ACK.
The pager's manual invocation registry previously distinguished RPC settlement but
not queued/executing phases. Automatic lifecycle notifications had no invocation
or prompt identity. Hidden thinking and early nonterminal tool updates could lose
semantic activity independently of actual backend progress.

## Decision

Extend the existing cancellation/compaction owner with an automatic invocation
scope carrying its prompt identity. Persist and forward this identity in additive
notification metadata. The existing pager tracker owns correlated live activity;
replay renders historical markers without acquiring that activity. The existing
manual invocation registry owns admission, queue, execution and RPC settlement.
Only the original manual RPC settles its command. Actual execution may acquire an
idle loader after a preceding foreground turn ends.

Keep model activity independent of thinking-block visibility. Merge early tool
updates using their actual status instead of treating every orphan as completed.
Validate prefire prefixes using their complete serialized content, including tool
arguments and images, while allowing appended tail items.

## Consequences

No scheduling mode, model, timeout, prompt cancellation policy or history format
is replaced. Parallel prefire, exclusive manual maintenance, predecessor ordering,
explicit stop, persistence barriers and diagnostic markers remain owned by their
existing implementations. Source regressions in `compaction_progress_tests.rs`
and `acp_handler/tests/progress_lifecycle.rs` exercise these retained capabilities
through session/ACP entry points and isolated inference fixtures. They are not
executed in this worktree.

Uncorrelated old live producers cannot provide invocation-level provenance.
Existing wire records remain loadable. Backend latency and runtime acceptance
require separate isolated evidence. Prefix hashing occurs only at prefire cache
creation/validation, not during rendering. Manual status reads inspect only the
current session's unsettled invocation registry, not conversation history.

## Status

Source implementation only. Compilation, tests, publication and runtime
acceptance are delegated to integration.
