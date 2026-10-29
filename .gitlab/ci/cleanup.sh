#!/usr/bin/env bash
# Pipeline-start cleanup: remove runner-side build state the next build does
# not need, before any validate/build job touches the shared caches.
#
# Scope (all project-scoped on the nspawn runner):
#   1. Leftover buildah working containers from this project (crashed jobs).
#   2. Dangling buildah layers/images (rebuilds recreate them from the
#      Containerfile; the tools image itself is rebuilt per job).
#   3. Cache files under /cache/grok-build/<project> untouched for CLEANUP_OLDER_THAN_DAYS
#      (stale dependency fingerprints, old incremental dirs, orphaned target
#      outputs from toolchain or dependency changes).
#   4. Stale dist/ leftovers in the persistent checkout (artifacts are already
#      uploaded to GitLab; a fresh build rewrites dist/).
#
# The cleanup job runs in the same resource_group as the build jobs, so no
# concurrent build can be reading the caches while they are pruned.
set -euo pipefail

: "${CI_PROJECT_ID:?GitLab project ID is required}"
: "${CI_PROJECT_DIR:?GitLab checkout is required}"

older_than_days="${CLEANUP_OLDER_THAN_DAYS:-14}"
cache_root="/cache/grok-build/${CI_PROJECT_ID}"

stopped_containers=$(buildah containers --format '{{.Containername}}' 2>/dev/null \
    | grep -x "grok-build-${CI_PROJECT_ID}-[0-9]*" || true)
if [[ -n "$stopped_containers" ]]; then
    printf 'Removing leftover buildah containers:\n%s\n' "$stopped_containers"
    buildah rm $stopped_containers || true
fi

printf 'Pruning dangling buildah layers/images\n'
buildah prune -f || true

if [[ -d "$cache_root" ]]; then
    printf "Pruning %s entries untouched for %s days\n" "$cache_root" "$older_than_days"
    find "$cache_root" -depth -type f -mtime "+$older_than_days" -delete
    find "$cache_root" -depth -type d -empty -mtime "+$older_than_days" -delete
else
    printf 'No cache root at %s yet\n' "$cache_root"
fi

if [[ -d "$CI_PROJECT_DIR/dist" ]]; then
    printf 'Removing stale dist/ leftovers from previous jobs\n'
    find "$CI_PROJECT_DIR/dist" -depth -type f -mtime "+$older_than_days" -delete
    find "$CI_PROJECT_DIR/dist" -depth -type d -empty -mtime "+$older_than_days" -delete
fi

printf 'Cleanup complete\n'
