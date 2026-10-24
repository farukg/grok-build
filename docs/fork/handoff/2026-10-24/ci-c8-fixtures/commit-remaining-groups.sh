#!/usr/bin/env bash
set -euo pipefail
root=/home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format
scratch=/tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures
python3 "$scratch/stage-group.py" pruning
git -C "$root" commit -F "$scratch/pruning-commit.txt"
python3 "$scratch/stage-group.py" pty
git -C "$root" commit -F "$scratch/pty-commit.txt"
git -C "$root" status --short
git -C "$root" log -6 --oneline
