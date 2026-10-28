#!/usr/bin/env bash

# Callers own command construction; listing and execution share one filter set.
run_test_target() {
    local evidence=$1 policy=$2 command_name=$3 filter result listing expected
    shift 3
    local compile_timeout=${test_compile_timeout:-60m} execution_timeout=${test_execution_timeout:-30m}
    local -n test_command=$command_name
    local -a filters=() libtest_args=(--test-threads=1)
    while (( $# > 0 )); do
        if [[ $1 == -- ]]; then
            shift
            libtest_args+=("$@")
            break
        fi
        filters+=("$1")
        shift
    done
    if (( ${#filters[@]} == 0 )); then
        filters=('')
    fi
    local exact=0 argument
    for argument in "${libtest_args[@]}"; do
        if [[ $argument == --exact ]]; then
            exact=1
        fi
    done
    local -a invocation=("${test_command[@]}" -- "${filters[@]}" "${libtest_args[@]}")
    listing=$(mktemp)
    printf '%q ' "${invocation[@]}" > "$evidence.command"
    printf '\n' >> "$evidence.command"
    if timeout --kill-after=30s "$compile_timeout" "${invocation[@]}" --list > "$listing" 2> "$evidence.compile.log"; then
        result=0
    else
        result=$?
    fi
    cp "$listing" "$evidence.list"
    cat "$listing" "$evidence.compile.log"
    printf 'compile_exit=%s\n' "$result" > "$evidence.result"
    if (( result != 0 )); then
        rm -f "$listing"
        return "$result"
    fi
    expected=$(awk '/: test$/ { count++ } END { print count + 0 }' "$listing")
    printf 'listed_tests=%s\n' "$expected" >> "$evidence.result"
    for filter in "${filters[@]}"; do
        if ! awk -v filter="$filter" -v exact="$exact" '/: test$/ { name = substr($0, 1, length($0) - 6); if ((exact && name == filter) || (!exact && index(name, filter))) found = 1 } END { exit !found }' "$listing"; then
            printf 'missing_required_filter=%s\n' "$filter" >> "$evidence.result"
            printf 'No compiled tests match required filter: %s / %s\n' "$evidence" "$filter" >&2
            rm -f "$listing"
            return 1
        fi
    done
    rm -f "$listing"
    if timeout --kill-after=30s "$execution_timeout" "${invocation[@]}" 2>&1 | tee "$evidence.run.log"; then
        printf 'execution_exit=0\n' >> "$evidence.result"
    else
        result=$?
        printf 'execution_exit=%s\n' "$result" >> "$evidence.result"
        return "$result"
    fi
    case "$policy" in
        all-executed)
            if ! awk -v expected="$expected" '$1 == "test" && $2 == "result:" && $3 == "ok." && $4 == expected && $6 == 0 && $8 == 0 { found = 1 } END { exit !found }' "$evidence.run.log"; then
                printf 'native_count_mismatch=1\n' >> "$evidence.result"
                printf 'Native target must execute every listed test without ignores.\n' >&2
                return 1
            fi
            ;;
        allow-ignored) ;;
        *) printf 'Unknown test result policy: %s\n' "$policy" >&2; return 2 ;;
    esac
}
