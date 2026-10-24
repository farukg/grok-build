#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/org/forks/grok-build
rustup run nightly-2026-08-13 rustfmt --edition 2024 --config skip_children=true crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs
git diff --cached --quiet
git add -- crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/finished_child_manual_compact_cancel.rs crates/codegen/xai-grok-pager-pty-harness/tests/pty_e2e/send_subagent_message_row.rs
git diff --cached --check
git diff --cached --stat
git commit --no-verify -F /tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/fix-oracles.txt
fix_commit=$(git rev-parse HEAD)
cd /home/fg/projects/worktrees.d/org/forks/grok-build@compaction-pty-format
test "$(git rev-parse HEAD)" = 02330d3ad44183d1de71cf885c6ac45813598234
test -z "$(git status --porcelain)"
git cherry-pick "$fix_commit"
cargo +nightly-2026-08-13 fmt -p xai-grok-sampler -p xai-grok-shell -p xai-grok-tools -p xai-grok-pager -p xai-grok-pager-pty-harness -- --check > /tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/format-ci-equivalent.log 2>&1
git diff --check
git rev-parse HEAD
