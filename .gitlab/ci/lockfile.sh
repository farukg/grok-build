#!/usr/bin/env bash
set -euo pipefail

: "${1:?Pinned toolchain is required from build-linux.sh}"
toolchain=$1
artifact=dist/lockfile
test -s "$artifact/provenance.txt"
test -s "$artifact/committed.Cargo.lock"
if ! cmp -s Cargo.lock "$artifact/committed.Cargo.lock"; then
    printf 'Lock generation requires the committed Cargo.lock as its starting point.\n' >&2
    exit 1
fi
trap 'cp "$artifact/committed.Cargo.lock" Cargo.lock' EXIT

{
    printf 'command=cargo +%s update --workspace\n' "$toolchain"
    cargo "+$toolchain" --version --verbose
    rustc "+$toolchain" --version --verbose
    sha256sum .gitlab-ci.yml .gitlab/ci/{Containerfile,build.sh,build-linux.sh,lockfile.sh} \
        rust-toolchain.toml "$artifact/committed.Cargo.lock"
} >> "$artifact/provenance.txt"

# --workspace preserves existing non-workspace locks; a bare update refreshes them.
if cargo "+$toolchain" update --workspace; then
    cp Cargo.lock "$artifact/Cargo.lock"
    (cd "$artifact" && sha256sum Cargo.lock > Cargo.lock.sha256)
else
    result=$?
    printf 'resolution_failed=%s\n' "$result" >> "$artifact/provenance.txt"
    printf 'Cargo lock resolution failed; no generated lock is approved for adoption.\n' >&2
    exit "$result"
fi

if ! cmp -s "$artifact/Cargo.lock" "$artifact/committed.Cargo.lock"; then
    printf 'comparison=different\n' >> "$artifact/provenance.txt"
    printf 'Cargo.lock is stale. Release/regression are blocked. Fetch this job artifact dist/lockfile/Cargo.lock, review and commit it, then rerun CI.\n' >&2
    exit 1
fi
printf 'comparison=identical\n' >> "$artifact/provenance.txt"
printf 'Cargo-generated lock matches the committed Cargo.lock.\n'
