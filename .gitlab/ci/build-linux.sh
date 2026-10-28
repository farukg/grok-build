#!/usr/bin/env bash
set -euo pipefail

cd /build
toolchain=$(sed -n 's/^channel = "\([^"]*\)"/\1/p' rust-toolchain.toml)
: "${toolchain:?Missing pinned Rust channel}"
rustup toolchain install "$toolchain" --profile minimal --no-self-update

case "${1:-release}" in
    release) ;;
    lockfile)
        exec bash .gitlab/ci/lockfile.sh "$toolchain"
        ;;
    format)
        rustup component add rustfmt --toolchain "$toolchain"
        exec python3 .gitlab/ci/format-source.py "$toolchain" "${@:2}"
        ;;
    tokio-checked-deadline)
        export RUST_MIN_STACK=16777216
        exec bash .gitlab/ci/tokio-checked-deadline.sh "$toolchain"
        ;;
    regression)
        export CARGO_TARGET_DIR=/cache/grok-build/target-regression
        export RUST_MIN_STACK=16777216
        exec bash .gitlab/ci/regression-tests.sh "$toolchain"
        ;;
    *)
        printf 'Unknown CI build mode: %s (expected release, lockfile, format, regression or tokio-checked-deadline)\n' "$1" >&2
        exit 2
        ;;
esac

# Fresh checkouts change mtimes. Content fingerprints preserve warm dependencies
# while detecting edits, and the release profile reuses incremental workspace code.
cargo "+$toolchain" -Z checksum-freshness build -p xai-grok-pager-bin --release --locked

mkdir -p dist
install "$CARGO_TARGET_DIR/release/xai-grok-pager" dist/grok
strip dist/grok
tar -C dist -czf dist/grok-x86_64-linux.tar.gz grok
(cd dist && sha256sum grok-x86_64-linux.tar.gz > grok-x86_64-linux.tar.gz.sha256)
