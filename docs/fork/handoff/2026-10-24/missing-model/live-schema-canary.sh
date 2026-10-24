#!/usr/bin/env bash
set -euo pipefail
python3 /tmp/grok-goal-4f7553a1dbe6/implementer/contracts-canary/probe.py http://127.0.0.1:6790 sigma/openai/gpt-6.1-sol schema-roundtrip > /tmp/grok-goal-4f7553a1dbe6/implementer/missing-model/live-schema-canary.log 2>&1
