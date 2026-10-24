# Compaction PTY and ChatState test-oracle review

## Finding

- **Severity:** P1 (the PTY regression can run compaction before the child is actually idle, so it does not reliably test the stated finished-child behavior)
- **§-Law:** §RUST-TESTING (behavior tests must assert the real contract through observable behavior)
- **File:** `/home/fg/projects/worktrees.d/org/forks/grok-build@compaction-pty-format/crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs:35`
- **Finding:** After opening the child, the test waits for `CHILD_DONE` but starts `/compact` without waiting for that child's running-to-idle transition; the supplied CI/source review states the fixture's queued interjection can cause a further child turn, and `CHILD_DONE` alone is not its completion barrier.
- **Fix:** Immediately after `scenario.wait_for_row("CHILD_DONE")`, call `scenario.harness.wait_for_turn_idle(ROW_TIMEOUT)` before arming the compaction hold and submitting `/compact`.

The cancellation oracle change is otherwise grounded: `CompactionCancelled` renders as `Compaction cancelled.` at `crates/codegen/xai-grok-pager/src/scrollback/blocks/session_event.rs:456`, while `Cancelling…` is transient status rendering. `WAKE_DONE` is the parent wake's response marker; `CHILD_DONE` comes from the child fixture conversation, so the revision fixes the wrong-screen marker but not the missing child-idle barrier.

## ChatState test-oracle assessment

The two-turn below-half case is a meaningful boundary oracle: 8,000-byte tool results estimate to 2,000 tokens each, making the retained history about 4,000 tokens against the 5,000 gate; the oldest result remains older than `keep_last_n_turns = 1` and exceeds the default 4,000-character trim threshold. It therefore detects an always-prune gate rather than merely testing an ineligible result.

The repeated 6,001 → 4,000 provider-report case uses the actual `ChatStateHandle` recording and request-pruning APIs, confirms the stored history remains unchanged, and checks the projected request trims the eligible result on each report. It is a useful regression for the documented retained-history policy (`actor/request_builder.rs:122–145`), not a full two-Responses sampler integration; the test's name and assertions claim repeated reports/projection rather than exercising sampler traffic. Static inspection found no apparent type/compile error in its new handle assertions. Build and execution were not verified.

## Reviewed scope and limits

Reviewed commit `0d565bc3` (`crates/codegen/xai-chat-state/src/actor/tests.rs`), PTY commit `6207646f` versus `02330d3a` (`finished_child_manual_compact_cancel.rs`, `send_subagent_message_row.rs`), and their owning request-pruning and PTY-idle source paths. No production pruning policy was changed or assessed for alteration. No build, check, test, or formatter was run. Runtime boundaries not verified: compilation, real test execution, child idle timing in CI, and PTY cancellation behavior against the built binary.
