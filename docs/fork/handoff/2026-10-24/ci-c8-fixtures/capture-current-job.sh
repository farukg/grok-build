#!/usr/bin/env bash
set -euo pipefail
gh api --allow-escape-sequences repos/farukg/grok-build/actions/jobs/120767075033/logs > /tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/partial-job-40058633091.raw
python3 /tmp/grok-goal-4f7553a1dbe6/implementer/pager-ci/strip-terminal-controls.py /tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/partial-job-40058633091.raw /tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/partial-job-40058633091.log
