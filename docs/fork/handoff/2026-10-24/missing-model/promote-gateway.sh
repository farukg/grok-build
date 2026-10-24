#!/usr/bin/env bash
set -euo pipefail
artifact=/home/fg/.cache/cargo-target/quick/release/sigma-gateway
expected=e212880479b2a050973fa992138212086ad2960684c19a55801c19df4a5919eb
test "$(sha256sum "$artifact" | cut -d ' ' -f 1)" = "$expected"
readlink -f /home/fg/.local/state/sigma-services/sigma-gateway/current
sig deploy stage sigma-gateway "$artifact"
pid=$(systemctl --user show sigma-gateway.service --property MainPID --value)
test "$pid" -gt 0
current=$(readlink -f /home/fg/.local/state/sigma-services/sigma-gateway/current)
running=$(readlink -f "/proc/$pid/exe")
test "$current" = "$running"
test "$(sha256sum "$current" | cut -d ' ' -f 1)" = "$expected"
sha256sum "$artifact" "$current" "/proc/$pid/exe"
python3 /tmp/grok-goal-4f7553a1dbe6/implementer/contracts-canary/probe.py http://127.0.0.1:6790 sigma/openai/gpt-6.1-sol schema-roundtrip
