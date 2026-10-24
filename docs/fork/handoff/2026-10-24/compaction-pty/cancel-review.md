# Follow-up PTY oracle review

## Finding: parent wake is not the child-finished barrier

The CI log confirms the child completed its shell command and emitted `CHILD_DONE`, while the parent's `PARENT_WAKE_DONE` appeared before the completion wake-up was delivered to the pager (`ci-39819829494-failed.log:5125-5238`). The updated flow correctly waits for `WAKE_DONE` on the parent, opens the child, and then waits for `CHILD_DONE` in the child view (`finished_child_manual_compact_cancel.rs:17-35`). This fixes the original wrong-screen oracle.

The fixture also sends three child messages, including an `interject` (`send_subagent_message_row.rs:50-85`). CI's request log shows that the interjection starts another child model turn after the held shell command exits; that later turn emits `CHILD_DONE` (`ci-39819829494-failed.log:5200-5238`). Consequently, seeing `CHILD_DONE` establishes visible assistant content but does not itself establish the final child turn has completed. The PTY harness documents `wait_for_turn_idle` as the completion barrier after rendered output (`xai-grok-pager-pty-harness/src/lib.rs:456-468`).

**Recommendation:** after `scenario.wait_for_row("CHILD_DONE")` in the opened child view, also await `scenario.harness.wait_for_turn_idle(ROW_TIMEOUT)` before `/compact`. This consumes the child’s actual running→idle terminal transition, handles the interjection follow-up, and does not extend any timeout. Keep the parent `WAKE_DONE` + parent idle wait: it proves that the parent’s wake turn has ended, while the child-view idle wait proves the child’s own work has settled.

## Fixture/oracle disposition

The fixture’s queued/interjected messages are incidental to the compaction test and increase the state transitions under review, but they do not invalidate the scenario once the child-view idle barrier is present. The smallest complete change is that additional idle wait; no larger fixture rewrite is required based on this source/log evidence. `PARENT_WAKE_DONE` alone would be insufficient because its script is a distinct parent conversation; it does not prove the final child notification has reached the parent row.

The test subsequently selects the same parent row (`CHILD_LABEL`) again after cancellation and reopens it (`finished_child_manual_compact_cancel.rs:93-107`). This establishes that the same transcript link remains usable, but it does not independently assert the raw session ID or tool-schema equality. Those stronger identity/capability contracts remain covered by the existing shell tests; the current terminal case makes no such equality claim.

## Scope

Source and supplied CI-log review only. No source edits, build, test, check, or CI action performed. Sources are under `/home/fg/projects/org/forks/grok-build/`.
