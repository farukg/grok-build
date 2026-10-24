# Finished-child manual compact cancellation PTY case

Authored a real ignored PTY e2e case using the existing child-session shell-tools fixture and its mock inference server. The scenario releases the fixture-owned child command to completion, opens the finished child from the parent transcript, starts `/compact`, holds the actual `xai-compact-` auxiliary inference reply, sends Ctrl+C through the PTY, releases the reply, and requires the rendered `Cancelling` state to disappear. It then opens `/session-info`, returns to the parent, and reopens the same child link. Session IDs and capability-schema equality are not asserted by this terminal scenario; the existing child lifecycle tests own those stronger invariants.

## Files

- `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs` — new PTY case.
- `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e_shell_tools.rs` — registers the case in the existing shell-tools PTY family.
- `crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs` — exposes the existing fixture's controller and hold flag to the sibling scenario; no fixture behavior changed.

## CI invocation

Use the already staged pager executable explicitly so harness binary resolution cannot fall back to a local build:

`PAGER_BINARY=/absolute/path/to/staged/xai-grok-pager cargo test -p xai-grok-pager-pty-harness --test pty_e2e_shell_tools finished_child_manual_compact_cancel -- --ignored`

The test is ignored for normal Cargo runs. Do not add `--exact` unless using the fully qualified test name emitted by libtest.

## Observations and limits

No local build, test, or check was run. The test exercises real pager terminal input/rendering with the existing mock server and owned disposable child fixture; it does not use the live leader/session. Cancellation uses the typed test-server auxiliary hold matched to the production `xai-compact-` request ID prefix. CI still needs to confirm the command's control-key routing, exact UI labels, finished-child selection row, and session-info/reopen behavior against the staged artifact. The scenario checks that `Cancelling` is eventually absent, not a particular cancellation marker or persistence identity; those guarantees remain covered by the existing shell/pager tests.
