#!/usr/bin/env bash
set -euo pipefail
sig build --project /home/fg/projects/org/platform/sigma-crates --context 'Urgent Responses envelope hotfix with pinned async-openai client and joined semantic-idle source' --requester Sokrates --subscriber Sokrates cargo test -p sigma-gateway --locked
