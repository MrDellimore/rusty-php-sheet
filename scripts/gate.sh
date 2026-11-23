#!/usr/bin/env bash
# The regression gate (`make gate`): every check from the implementation plan that
# can run locally, then a summary to paste into the phase's outcome block.
#
#   1 lint            clippy, rustfmt, stub syntax
#   2 coverage        Rust + PHP tests on an instrumented build; targets, ratchet, exclusions
#   3 parity          (part of 2: the Pest parity suites)
#   4 fuzz            every cargo-fuzz target, FUZZ_SECONDS each (default 60)
#   5 bench           400k-row read/write against php/bench/baseline.json
#   6 sheet-stream    its suite with the rust drivers, then with OpenSpout
#                     (skipped when SHEET_STREAM, default ../laravel-sheet-stream, is absent)
#
# GATE_SKIP="fuzz bench" skips steps (say why in the outcome block).
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PHP="${PHP:-php}"
SHEET_STREAM="${SHEET_STREAM:-$ROOT/../laravel-sheet-stream}"
case "$(uname -s)" in Darwin) EXT=dylib ;; *) EXT=so ;; esac
LOG="$ROOT/target/gate"
mkdir -p "$LOG"
SUMMARY=()
FAILED=0

step() {
    local name=$1
    shift
    if [[ " ${GATE_SKIP:-} " == *" $name "* ]]; then
        SUMMARY+=("$(printf '%-13s skipped (GATE_SKIP)' "$name")")
        return
    fi
    echo "==> $name"
    local start=$SECONDS
    if "$@" >"$LOG/$name.log" 2>&1; then
        SUMMARY+=("$(printf '%-13s pass  %4ss  %s' "$name" $((SECONDS - start)) "$(summary_of "$name")")")
    else
        FAILED=1
        SUMMARY+=("$(printf '%-13s FAIL  %4ss  see target/gate/%s.log' "$name" $((SECONDS - start)) "$name")")
        tail -30 "$LOG/$name.log"
    fi
}

summary_of() {
    local f="$LOG/$1.log"
    case $1 in
        coverage)
            local rust php cov
            rust=$(grep -Eo 'test result: ok\. [0-9]+ passed' "$f" | awk '{s += $4} END {print s}')
            php=$(grep -Eo '[0-9]+ passed \([0-9]+ assertions\)' "$f" | tail -1)
            cov=$(grep -E '^  rusty_sheet' "$f" | awk '{printf "%s %s ", $1, $3}')
            echo "rust ${rust} passed, php ${php}; lines: ${cov}" ;;
        fuzz) grep -E '^Fuzzing passed' "$f" ;;
        bench) grep -E '^(read|write)-' "$f" | awk '{printf "%s %s/s; ", $1, $2}' ;;
        sheet-stream) grep -Eo '[0-9]+ passed' "$f" | paste -sd' ' - | sed 's/^/rust drivers, then OpenSpout: /' ;;
        *) echo "ok" ;;
    esac
}

sheet_stream() {
    [ -d "$SHEET_STREAM" ] || { echo "no sheet-stream checkout at $SHEET_STREAM"; return 0; }
    cargo build --quiet -p rusty_sheet
    local ext="$ROOT/target/debug/librusty_sheet.$EXT"
    (cd "$SHEET_STREAM" && SHEET_STREAM_TEST_READER=rust SHEET_STREAM_TEST_WRITER=rust \
        "$PHP" -d "extension=$ext" vendor/bin/pest --colors=never) &&
    (cd "$SHEET_STREAM" && "$PHP" -d "extension=$ext" vendor/bin/pest --colors=never)
}

bench() {
    cargo build --quiet --release -p rusty_sheet
    [ -f tests/fixtures/generated/large-400000.xlsx ] || make --no-print-directory large-fixture ROWS=400000
    "$PHP" php/bench/check.php "$ROOT/target/release/librusty_sheet.$EXT" tests/fixtures/generated/large-400000.xlsx
}

step lint make --no-print-directory lint
step coverage ./scripts/coverage.sh
step fuzz ./scripts/fuzz.sh
step bench bench
step sheet-stream sheet_stream

echo
echo "Regression gate ($(date '+%Y-%m-%d'), $(git rev-parse --short HEAD 2>/dev/null || echo 'no git')):"
printf '  %s\n' "${SUMMARY[@]}"
exit $FAILED
