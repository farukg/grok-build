# Cargo tool-family CI delivery

## Current contract

The inner Grok CI image consumes artifacts from the successful `publish-cargo-release`
job in the `publish` stage of `org/platform/sigma-crates` (project 28):

- `ci-artifacts/cargo-localbuild/family-archive.tar`
- `ci-artifacts/cargo-localbuild/family-complete.toml`

The build pins the project, pipeline, job, full source commit, job and pipeline URLs,
archive digest, Cargo-fork digest and seal-file digest. `build.sh` writes the CI job
token to a temporary mode-600 file, removes it on exit, and passes it only as a
Buildah secret. The verifier checks job and pipeline status and provenance before
fetching artifacts, then verifies archive and seal digests, fork identity, seal
fields, and every archive member's path, mode, size and digest.

The producer's seven executable members are `cargo-localbuild`, `cargo-quick`,
`cargo-perf`, `cargo-size`, `cargo-release`, `cargo-remote-bld` and `sig`. The
archive also contains `resolution/` files including the vendor closure, `source.tar`,
`sidecar.tar`, `SOURCE.txt`, `help.txt` and `SHA256SUMS`. The producer excludes
`SHA256SUMS` from its checksum-declared expected set but includes it in the seal
and archive inventory. Executables are installed under `/opt/sigma/cargo-localbuild/bin`
and linked from `/usr/local/bin`; `cargo` remains the rustup proxy.

The seven cache-retention values remain `1`; persistent cache configuration is
unchanged.

## Evidence and blockers

Grok main `357c8fd9877ad7142072772f942572470fc766d1`, pipeline `11093`,
cleanup job `86673`, failed because `cargo-release` was absent from inner image
`6b2d665fc198` (exit 127). Subsequent lockfile, format, release and regression
stages were skipped.

Sigma-Crates pipeline `11094`, producer job `86689` failed. No qualified archive,
seal or resulting inner-image digest is available. Qualification requires a
successful producer with exact identity and hashes supplied to Grok CI, followed
by successful GitLab acceptance of the consumer. Source-only syntax checks do not
establish that acceptance.
