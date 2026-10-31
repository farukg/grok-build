#!/usr/bin/env bash
set -euo pipefail

: "${CI_PROJECT_ID:?GitLab project ID is required}"
if [[ ! "$CI_PROJECT_ID" =~ ^[1-9][0-9]*$ ]]; then
    printf 'Invalid GitLab project ID; cleanup refused.\n' >&2
    exit 1
fi

exec bash "${CI_PROJECT_DIR:?GitLab checkout is required}/.gitlab/ci/build.sh" cleanup
