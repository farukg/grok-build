#!/usr/bin/env bash
set -euo pipefail
gh run view 39873008176 --job 120151742235 --repo farukg/grok-build --log | grep --line-buffered 'Test finished child compaction through the terminal' > /tmp/grok-goal-4f7553a1dbe6/implementer/compaction-pty/pty-success-a0cbf7c4.log
