#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/org/forks/grok-build
rustup run nightly-2026-08-13 rustfmt --edition 2024 --config skip_children=true crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs
cmp -s crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs /home/fg/projects/worktrees.d/org/forks/grok-build@compaction-pty-format/crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs
git diff --check -- crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs
git diff --stat -- crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs
git commit --no-verify --only -F /tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/main-pty-commit.txt -- crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs
git rev-parse HEAD
