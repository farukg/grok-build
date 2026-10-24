#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/org/governance/sigma
exec /home/fg/.cache/cargo-target/quick/release/sigma-gateway --host 127.0.0.1 --port 6796 --log-level warn > /tmp/grok-goal-4f7553a1dbe6/implementer/missing-model/canary-server.log 2>&1
