#!/usr/bin/env bash
set -euo pipefail

: "${CI_PROJECT_ID:?GitLab project ID is required}"
: "${CI_JOB_ID:?GitLab job ID is required}"
: "${CI_PROJECT_DIR:?GitLab checkout is required}"

cd "$CI_PROJECT_DIR"
: "${CI_COMMIT_SHA:?GitLab source SHA is required}"
if [[ $(git rev-parse HEAD) != "$CI_COMMIT_SHA" ]] || ! git diff --quiet HEAD -- Cargo.lock; then
    printf 'CI requires the exact source checkout and its committed Cargo.lock.\n' >&2
    exit 1
fi

if [[ ${1:-release} == lockfile ]]; then
    : "${CI_PIPELINE_ID:?GitLab pipeline ID is required}"
    mkdir -p dist/lockfile
    git show HEAD:Cargo.lock > dist/lockfile/committed.Cargo.lock
    printf 'project_id=%s\npipeline_id=%s\njob_id=%s\ncommit_sha=%s\n' \
        "$CI_PROJECT_ID" "$CI_PIPELINE_ID" "$CI_JOB_ID" "$CI_COMMIT_SHA" \
        > dist/lockfile/provenance.txt
fi

if [[ ${1:-release} == format ]]; then
    : "${CI_PIPELINE_ID:?GitLab pipeline ID is required}"
    mkdir -p dist/format
    printf 'project_id=%s\npipeline_id=%s\njob_id=%s\ncommit_sha=%s\n' \
        "$CI_PROJECT_ID" "$CI_PIPELINE_ID" "$CI_JOB_ID" "$CI_COMMIT_SHA" \
        > dist/format/provenance.txt
    set -- format "${CI_PIPELINE_SOURCE:?GitLab pipeline source is required}" \
        "${CI_COMMIT_BEFORE_SHA:-}" "${FORMAT_BASE_SHA:-}" "$CI_COMMIT_SHA"
fi

if [[ ${1:-release} == regression ]]; then
    : "${CI_PIPELINE_ID:?GitLab pipeline ID is required}"
    mkdir -p dist/regression
    printf 'project_id=%s\npipeline_id=%s\njob_id=%s\ncommit_sha=%s\n' \
        "$CI_PROJECT_ID" "$CI_PIPELINE_ID" "$CI_JOB_ID" "$CI_COMMIT_SHA" \
        > dist/regression/provenance.txt
fi

if [[ ${1:-release} == tokio-checked-deadline ]]; then
    : "${CI_PIPELINE_ID:?GitLab pipeline ID is required}"
    mkdir -p dist/tokio-checked-deadline
    printf 'project_id=%s\npipeline_id=%s\njob_id=%s\ncommit_sha=%s\n' \
        "$CI_PROJECT_ID" "$CI_PIPELINE_ID" "$CI_JOB_ID" "$CI_COMMIT_SHA" \
        > dist/tokio-checked-deadline/provenance.txt
fi

image="localhost/grok-build-ci:${CI_PROJECT_ID}"
container="grok-build-${CI_PROJECT_ID}-${CI_JOB_ID}"
cache_root="/cache/grok-build/${CI_PROJECT_ID}"

# The nspawn runner binds /cache and container storage across jobs on this host.
# Only the small, secret-free tools image uses layer caching; source stays a bind mount.
buildah bud --network=host --layers --pull=missing --tag "$image" \
    --file .gitlab/ci/Containerfile .gitlab/ci
mkdir -p "$cache_root"
buildah from --name "$container" "$image"
trap 'buildah rm "$container"' EXIT
buildah run --network=host \
    --env "CARGO_LOCALBUILD_DEPS_VARIANT_MIN_AGE_SECS=${CARGO_LOCALBUILD_DEPS_VARIANT_MIN_AGE_SECS:?Retention value is required}" \
    --env "CARGO_LOCALBUILD_DEPS_USE_GUARD_SECS=${CARGO_LOCALBUILD_DEPS_USE_GUARD_SECS:?Retention value is required}" \
    --env "CARGO_LOCALBUILD_KEEP_FITTEST_DEPS_VARIANTS=${CARGO_LOCALBUILD_KEEP_FITTEST_DEPS_VARIANTS:?Retention value is required}" \
    --env "CARGO_LOCALBUILD_INCREMENTAL_SESSION_MAX_AGE_SECS=${CARGO_LOCALBUILD_INCREMENTAL_SESSION_MAX_AGE_SECS:?Retention value is required}" \
    --env "CARGO_LOCALBUILD_ARTIFACT_ORPHAN_MIN_AGE_SECS=${CARGO_LOCALBUILD_ARTIFACT_ORPHAN_MIN_AGE_SECS:?Retention value is required}" \
    --env "CARGO_LOCALBUILD_KEEP_RECENT_BUILD_VARIANTS=${CARGO_LOCALBUILD_KEEP_RECENT_BUILD_VARIANTS:?Retention value is required}" \
    --env "CARGO_LOCALBUILD_CONFIGURATION_IDLE_SECS=${CARGO_LOCALBUILD_CONFIGURATION_IDLE_SECS:?Retention value is required}" \
    --volume "$CI_PROJECT_DIR:/build:rw" \
    --volume "$cache_root:/cache/grok-build:rw" \
    "$container" -- bash .gitlab/ci/build-linux.sh "$@"
