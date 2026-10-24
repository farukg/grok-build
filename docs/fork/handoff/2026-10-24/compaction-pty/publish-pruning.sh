#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/worktrees.d/org/forks/grok-build@compaction-pty-format
rustup run nightly-2026-08-13 rustfmt --edition 2024 --config skip_children=true crates/codegen/xai-chat-state/src/actor/tests.rs
git diff --cached --quiet
git add -- crates/codegen/xai-chat-state/src/actor/tests.rs
git diff --cached --check
git diff --cached --stat
git commit --no-verify -F /tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/pruning-commit.txt
git rev-parse HEAD
sig push --force-non-mic --push-anyway origin sokrates/compaction-pty-c0e3db3f
gh workflow run linux-test.yml --repo farukg/grok-build --ref sokrates/compaction-pty-c0e3db3f
gh workflow run linux-build.yml --repo farukg/grok-build --ref sokrates/compaction-pty-c0e3db3f
