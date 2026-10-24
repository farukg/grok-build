#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format
test -z "$(git status --porcelain)"
git merge-base --is-ancestor origin/main HEAD
exec sig push --force-non-mic --push-anyway origin HEAD:refs/heads/main
