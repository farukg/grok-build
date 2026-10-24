#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/worktrees.d/org/forks/grok-build@compaction-pty-format
git diff --check -- .github/workflows/linux-test.yml
git diff -- .github/workflows/linux-test.yml
git commit --no-verify --only -F /tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/cache-commit.txt -- .github/workflows/linux-test.yml
git rev-parse HEAD
sig push --force-non-mic --push-anyway origin sokrates/compaction-pty-c0e3db3f
