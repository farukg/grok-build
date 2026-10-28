#!/usr/bin/env bash
set -euo pipefail

: "${1:?Pinned toolchain is required from build-linux.sh}"
toolchain=$1
source .gitlab/ci/test-target.sh
test_compile_timeout=20m
test_execution_timeout=5m
output=/build/dist/tokio-checked-deadline
vendor=$(realpath third_party/tokio)
manifest="$vendor/Cargo.toml"
mkdir -p "$output"
[[ -f $vendor/src/time/checked_deadline.rs ]] || {
    printf 'CheckedDeadline source is required; this mode cannot validate pristine Tokio.\n' >&2
    exit 1
}
target=$(rustc "+$toolchain" -vV | sed -n 's/^host: //p')
: "${target:?Missing native compiler target}"
patch="patch.crates-io.tokio.path=\"$vendor\""
base=(cargo "+$toolchain" --config "$patch")
{
    printf 'source=%s\ntarget=%s\nstandalone_patch=%s\n' "$vendor" "$target" "$vendor"
    cargo "+$toolchain" --version
    rustc "+$toolchain" -vV
    sha256sum .gitlab-ci.yml .gitlab/ci/{Containerfile,build.sh,build-linux.sh,test-target.sh,tokio-checked-deadline.sh,tokio-metadata.py} rust-toolchain.toml .cargo/config.toml
    find "$vendor/src" "$vendor/tests" -type f -print0 | sort -z | xargs -0 sha256sum
    sha256sum "$manifest"
} >> "$output/provenance.txt"

# Excluded workspace: Cargo generates this dev-graph lock remotely, never the root lock.
printf '%q ' "${base[@]}" generate-lockfile --manifest-path "$manifest" > "$output/lock.command"
printf '\n' >> "$output/lock.command"
if timeout --kill-after=30s 15m "${base[@]}" generate-lockfile --manifest-path "$manifest" > "$output/lock.log" 2>&1; then
    printf 'generation_exit=0\n' > "$output/lock.result"
else
    result=$?
    cat "$output/lock.log"
    printf 'generation_exit=%s\n' "$result" > "$output/lock.result"
    exit "$result"
fi
cp "$vendor/Cargo.lock" "$output/Cargo.lock"
(cd "$output" && sha256sum Cargo.lock > Cargo.lock.sha256)

run_flavor() (
    local flavor=$1 features=$2 result target_name
    export CARGO_TARGET_DIR="/cache/grok-build/target-tokio-checked-deadline-$flavor"
    local evidence="$output/$flavor"
    mkdir -p "$evidence"
    if [[ $flavor == unstable-tracing || $flavor == time-no-rt-tracing ]]; then
        # RUSTFLAGS overrides Cargo target flags; preserve the configured native flags.
        CARGO_ENCODED_RUSTFLAGS=$(python3 .gitlab/ci/tokio-metadata.py flags .cargo/config.toml "$target") || return $?
        export CARGO_ENCODED_RUSTFLAGS
    fi
    printf 'flavor=%s\nfeatures=%s\nsource=%s\ntarget=%s\ncache=%s\n' \
        "$flavor" "$features" "$vendor" "$target" "$CARGO_TARGET_DIR" > "$evidence/identity.txt"
    printf '%s' "${CARGO_ENCODED_RUSTFLAGS-${RUSTFLAGS-}}" | sha256sum >> "$evidence/identity.txt"
    local -a options=(--manifest-path "$manifest" --locked --no-default-features --features "$features")
    local -a metadata=("${base[@]}" metadata "${options[@]}" --format-version 1 --filter-platform "$target")
    printf '%q ' "${metadata[@]}" > "$evidence/metadata.command"
    printf '\n' >> "$evidence/metadata.command"
    if timeout --kill-after=30s 15m "${metadata[@]}" > "$evidence/metadata.json" 2> "$evidence/metadata.log"; then
        printf 'metadata_exit=0\n' > "$evidence/metadata.result"
    else
        result=$?
        printf 'metadata_exit=%s\n' "$result" > "$evidence/metadata.result"
        cat "$evidence/metadata.log"
        return "$result"
    fi
    if python3 .gitlab/ci/tokio-metadata.py graph "$evidence/metadata.json" "$target" "$manifest" > "$evidence/graph.txt" 2> "$evidence/graph.log"; then
        printf 'graph_exit=0\n' >> "$evidence/metadata.result"
    else
        result=$?
        printf 'graph_exit=%s\n' "$result" >> "$evidence/metadata.result"
        cat "$evidence/graph.log"
        return "$result"
    fi
    local -a command=("${base[@]}" -Z checksum-freshness check "${options[@]}" --target "$target" --lib)
    printf '%q ' "${command[@]}" > "$evidence/check.command"
    printf '\n' >> "$evidence/check.command"
    if timeout --kill-after=30s 30m "${command[@]}" > "$evidence/check.log" 2>&1; then
        printf 'compile_exit=0\nruntime_asserted=no\n' > "$evidence/check.result"
    else
        result=$?
        printf 'compile_exit=%s\nruntime_asserted=no\n' "$result" > "$evidence/check.result"
        cat "$evidence/check.log"
        return "$result"
    fi
    case "$flavor" in
        time | time-rt | time-no-rt-tracing) return 0 ;;
        full | unstable-tracing) ;;
        *) printf 'Unknown Tokio flavor: %s\n' "$flavor" >&2; return 2 ;;
    esac
    command=("${base[@]}" -Z checksum-freshness test "${options[@]}" --target "$target" --lib)
    local -a filters=(
        'time::checked_deadline::tests::'
        'time::checked_deadline::wait_tests::'
        'time::checked_deadline::tests::old_runtime_checks_its_real_epoch_and_rounding_at_the_tick_limit'
        'time::checked_deadline::tests::actual_instant_rounding_overflow_is_typed'
    )
    if [[ $flavor == unstable-tracing ]]; then
        filters+=('time::checked_deadline::tests::alternative_timer_wait_keeps_its_owner_when_polled_elsewhere')
    fi
    local status=0
    run_test_target "$evidence/checked-deadline" all-executed command "${filters[@]}" || status=$?
    for target_name in time_sleep time_interval time_timeout; do
        command=("${base[@]}" -Z checksum-freshness test "${options[@]}" --target "$target" --test "$target_name")
        run_test_target "$evidence/$target_name" all-executed command || status=$?
    done
    return "$status"
)

status=0
run_flavor time time || status=$?
run_flavor time-rt time,rt || status=$?
run_flavor time-no-rt-tracing time,tracing || status=$?
run_flavor full full,test-util || status=$?
run_flavor unstable-tracing full,test-util,tracing || status=$?
printf 'attempted_flavors=5\nexecution_exit=%s\n' "$status" > "$output/summary.txt"
exit "$status"
