#!/usr/bin/env bash
set -euo pipefail

cd /build
toolchain=$(sed -n 's/^channel = "\([^"]*\)"/\1/p' rust-toolchain.toml)
: "${toolchain:?Missing pinned Rust channel}"
rustup toolchain install "$toolchain" --profile minimal --no-self-update

# Fresh checkouts change mtimes. Content fingerprints preserve warm dependencies
# while detecting edits, and the release profile reuses incremental workspace code.
cargo "+$toolchain" -Z checksum-freshness build -p xai-grok-pager-bin --release --locked

mkdir -p dist
install "$CARGO_TARGET_DIR/release/xai-grok-pager" dist/grok
strip dist/grok
tar -C dist -czf dist/grok-x86_64-linux.tar.gz grok
(cd dist && sha256sum grok-x86_64-linux.tar.gz > grok-x86_64-linux.tar.gz.sha256)
