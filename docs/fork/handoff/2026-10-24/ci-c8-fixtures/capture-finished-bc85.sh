#!/usr/bin/env bash
set -euo pipefail
gh run view 40058633091 --repo farukg/grok-build --log > /tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/bc85-failed-ci.raw
python3 /tmp/grok-goal-4f7553a1dbe6/implementer/pager-ci/strip-terminal-controls.py /tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/bc85-failed-ci.raw /tmp/grok-goal-4f7553a1dbe6/implementer/ci-c8-fixtures/bc85-failed-ci.log
