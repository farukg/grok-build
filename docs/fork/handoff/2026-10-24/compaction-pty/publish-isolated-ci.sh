#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/worktrees.d/org/forks/grok-build@compaction-pty-format
test "$(git rev-parse HEAD)" = 6207646f4d41d8534a6482604315483db47f4351
test -z "$(git status --porcelain)"
git switch sokrates/compaction-pty-c0e3db3f
git branch --set-upstream-to=origin/main sokrates/compaction-pty-c0e3db3f
sig push --force-non-mic --push-anyway origin sokrates/compaction-pty-c0e3db3f
gh workflow run linux-test.yml --repo farukg/grok-build --ref sokrates/compaction-pty-c0e3db3f
gh workflow run linux-build.yml --repo farukg/grok-build --ref sokrates/compaction-pty-c0e3db3f
gh run list --repo farukg/grok-build --commit 6207646f4d41d8534a6482604315483db47f4351 --limit 10 --json databaseId,workflowName,headSha,status,conclusion,url
