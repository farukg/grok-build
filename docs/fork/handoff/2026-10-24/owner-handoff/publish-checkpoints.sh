#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format
test "$(git rev-parse refs/heads/handoff/grok-canonical-20261024)" = 074f54b599d2bdd46c150124de4dc5f117641565
test "$(git rev-parse refs/heads/handoff/grok-runtime-20261024)" = 14a5f56c12f423f58540f649360fc10a115c92f0
exec sig push --force-non-mic origin refs/heads/handoff/grok-canonical-20261024:refs/heads/handoff/grok-canonical-20261024 refs/heads/handoff/grok-runtime-20261024:refs/heads/handoff/grok-runtime-20261024
