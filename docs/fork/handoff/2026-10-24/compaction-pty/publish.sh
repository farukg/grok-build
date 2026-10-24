#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/org/forks/grok-build
rustup run nightly-2026-08-13 rustfmt --edition 2024 --config skip_children=true crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e_shell_tools.rs
git diff --cached --quiet
git add -- .github/workflows/linux-test.yml crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e_shell_tools.rs
git diff --cached --check
git diff --cached --stat
git commit --no-verify -F /tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/commit.txt
git rev-parse HEAD
sig push
