#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/org/forks/grok-build
test "$(git rev-parse HEAD)" = c0e3db3ff50410c7d1b4abc8b17beb346b5b2884
sig push --force-non-mic --push-anyway
gh workflow run linux-test.yml --repo farukg/grok-build --ref main
gh run list --repo farukg/grok-build --commit c0e3db3ff50410c7d1b4abc8b17beb346b5b2884 --limit 10 --json databaseId,workflowName,headSha,status,conclusion,url
