#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/worktrees.d/org/forks/grok-build@compaction-pty-format
rustup run nightly-2026-08-13 cargo fmt -p xai-grok-sampler -p xai-grok-shell -p xai-grok-tools -p xai-grok-pager -p xai-grok-pager-pty-harness -- --check > /tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/format-ci-equivalent.log 2>&1
git diff --check
git rev-parse HEAD
