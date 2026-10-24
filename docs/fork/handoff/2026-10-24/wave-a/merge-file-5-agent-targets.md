Status: done.

Per-region decisions:
- Lines 571–591: retained the canonical target-test rewrite, `finish_child_with_result_before_visibility(&mut c, IDS[3], failed)`, in place of the runtime direct `begin_terminalization`/`ChildRunOutput` sequence. This preserves the test's failed-result setup and publication/replay assertions while using the canonical helper and its implementation-independent fixture boundary.

Open questions: none.
