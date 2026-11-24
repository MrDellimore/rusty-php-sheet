#!/usr/bin/env bash
# Run every cargo-fuzz target for FUZZ_SECONDS (default 60) each (`make fuzz`).
#
# Corpora start from the committed fixtures: whole workbooks for `xlsx`, their
# extracted parts for `sheet` / `shared_strings` / `styles`, and the CSV/TSV files
# for `csv`. They live in fuzz/corpus (gitignored) and grow across runs.
# A crash, a hang (> 10 s on one input) or > 2 GB of memory fails the run; the
# input is saved in fuzz/artifacts/<target>/ and must become a regression test.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECONDS_PER_TARGET="${FUZZ_SECONDS:-60}"
TARGETS=(sheet shared_strings styles csv xlsx)
FIXTURES="$ROOT/tests/fixtures"
cd "$ROOT/fuzz"

seed() {
    local target=$1 dir="corpus/$1"
    mkdir -p "$dir"
    [ -n "$(ls -A "$dir")" ] && return
    local i=0
    case $target in
        xlsx)
            find "$FIXTURES/edge" "$FIXTURES/laravel-excel" -name '*.xlsx' -size -512k -exec cp {} "$dir/" \; ;;
        csv)
            for f in "$FIXTURES"/edge/*.{csv,tsv} "$FIXTURES"/laravel-excel/*.{csv,tsv}; do
                [ -f "$f" ] || continue
                # The first byte picks delimiter, enclosure, escape and empty-row mode.
                { printf '\x00'; cat "$f"; } > "$dir/$(basename "$f")"
            done ;;
        *)
            local part
            case $target in
                sheet) part='xl/worksheets/sheet*.xml' ;;
                shared_strings) part='xl/sharedStrings.xml' ;;
                styles) part='xl/styles.xml' ;;
            esac
            for f in "$FIXTURES"/edge/*.xlsx "$FIXTURES"/laravel-excel/*.xlsx; do
                i=$((i + 1))
                unzip -p "$f" "$part" > "$dir/$i.xml" 2>/dev/null || true
                [ -s "$dir/$i.xml" ] || rm -f "$dir/$i.xml"
            done ;;
    esac
}

# Build for the host triple: a prebuilt cargo-fuzz (CI installs one) otherwise
# defaults to the triple it was itself built for, e.g. x86_64-unknown-linux-musl.
TARGET="$(rustc +nightly -vV | sed -n 's/^host: //p')"
cargo +nightly fuzz build --release --target "$TARGET" >/dev/null
for t in "${TARGETS[@]}"; do
    seed "$t"
    echo "==> fuzz $t (${SECONDS_PER_TARGET}s, $(ls "corpus/$t" | wc -l | tr -d ' ') seeds)"
    cargo +nightly fuzz run --target "$TARGET" "$t" "corpus/$t" -- \
        -max_total_time="$SECONDS_PER_TARGET" -timeout=10 -rss_limit_mb=2048 \
        -print_final_stats=1 2>&1 | grep -E 'stat::number_of_executed_units|SUMMARY|panicked|ERROR|Test unit written' || true
    if [ -n "$(ls -A "artifacts/$t" 2>/dev/null)" ]; then
        echo "FAIL: fuzz/artifacts/$t has crashing inputs"
        exit 1
    fi
done
echo "Fuzzing passed: ${#TARGETS[@]} targets × ${SECONDS_PER_TARGET}s."
