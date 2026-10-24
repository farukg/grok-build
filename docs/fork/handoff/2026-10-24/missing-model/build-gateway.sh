#!/usr/bin/env bash
set -euo pipefail
sig build --project /home/fg/projects/org/platform/sigma-crates --context 'Build verified Responses envelope hotfix for immediate canary and live promotion' --requester Sokrates --subscriber Sokrates cargo build -p sigma-gateway --bin sigma-gateway --locked
