#!/usr/bin/env bash
set -euo pipefail
cd /home/fg/projects/worktrees.d/org/forks/grok-build@ci-c8-format
scratch=/tmp/grok-goal-4f7553a1dbe6/implementer/ci-bc85-runtime
python3 "$scratch/format-regions.py"
mapfile -t paths < "$scratch/format-paths.txt"
rustup run nightly-2026-08-13 rustfmt --unstable-features --edition 2024 --config-path rustfmt.toml --config skip_children=true --file-lines "$(cat "$scratch/format-lines.json")" "${paths[@]}"
