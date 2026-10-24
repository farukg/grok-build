#!/usr/bin/env bash
set -euo pipefail
sig build --project /home/fg/projects/org/platform/sigma-crates --context 'Resolve the pinned async-openai dev-only consumer dependency in the host lockfile' --requester Sokrates --subscriber Sokrates cargo update -p async-openai
