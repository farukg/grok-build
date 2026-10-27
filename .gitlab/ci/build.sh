#!/usr/bin/env bash
set -euo pipefail

: "${CI_PROJECT_ID:?GitLab project ID is required}"
: "${CI_JOB_ID:?GitLab job ID is required}"
: "${CI_PROJECT_DIR:?GitLab checkout is required}"

cd "$CI_PROJECT_DIR"
image="localhost/grok-build-ci:${CI_PROJECT_ID}"
container="grok-build-${CI_PROJECT_ID}-${CI_JOB_ID}"
cache_root="/cache/grok-build/${CI_PROJECT_ID}"

# The nspawn runner binds /cache and container storage across jobs on this host.
# Only the small, secret-free tools image uses layer caching; source stays a bind mount.
buildah bud --layers --pull=missing --tag "$image" \
    --file .gitlab/ci/Containerfile .gitlab/ci
mkdir -p "$cache_root"
buildah from --name "$container" "$image"
trap 'buildah rm "$container"' EXIT
buildah run \
    --volume "$CI_PROJECT_DIR:/build:rw" \
    --volume "$cache_root:/cache/grok-build:rw" \
    "$container" -- bash .gitlab/ci/build-linux.sh
