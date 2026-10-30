#!/usr/bin/env bash
set -euo pipefail

: "${CI_PROJECT_ID:?GitLab project ID is required}"
if [[ ! "$CI_PROJECT_ID" =~ ^[1-9][0-9]*$ ]]; then
    printf 'Invalid GitLab project ID; cleanup refused.\n' >&2
    exit 1
fi

# The GitLab resource group does not settle orphaned executor writers.
printf '%s\n' \
    'Cleanup refused: nspawn executor writer settlement is not established.' \
    'The executor owner must guarantee writer termination before container reclaim.' \
    'Cargo-aware cleanup also requires delivered Cargo fork and cargo-localbuild tools in both CI container layers.' \
    'No project containers, images, caches or dist artifacts were removed by this script.' >&2
exit 1
