#!/usr/bin/env bash
# One coverage report across Rust and PHP (`make coverage`).
#
# The binding crate (`rusty_sheet`) is only exercised through PHP, so the extension
# is built with coverage instrumentation and the Pest suite runs with it loaded.
# Every process (cargo tests, php) writes .profraw files that are merged into one
# report: target/coverage/{lcov.info,html/,summary.json}.
#
# Instrumented builds go to their own target directory so they never replace the
# normal debug/release extension that other projects load.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PHP="${PHP:-php}"
OUT="$ROOT/target/coverage"
case "$(uname -s)" in Darwin) EXT=dylib ;; *) EXT=so ;; esac

export CARGO_TARGET_DIR="$ROOT/target/llvm-cov-target"
# Code that exists only to drive tests or benchmarks is not measured.
IGNORE='(tools/|benches/|examples/|src/fuzzing\.rs|src/xlsx/read_paths\.rs|/tests/|\.cargo/registry)'

eval "$(cargo llvm-cov show-env --sh 2>/dev/null)"
cargo llvm-cov clean --workspace

echo "==> Rust tests (instrumented)"
cargo test --workspace --quiet

echo "==> PHP tests with the instrumented extension"
cargo build --quiet -p rusty_sheet
(cd php && "$PHP" -d "extension=$CARGO_TARGET_DIR/debug/librusty_sheet.$EXT" vendor/bin/pest --colors=never)

echo "==> Report"
mkdir -p "$OUT"
cargo llvm-cov report --ignore-filename-regex "$IGNORE" --lcov --output-path "$OUT/lcov.info"
cargo llvm-cov report --ignore-filename-regex "$IGNORE" --html --output-dir "$OUT"
cargo llvm-cov report --ignore-filename-regex "$IGNORE" --json --summary-only --output-path "$OUT/summary.json"
cargo llvm-cov report --ignore-filename-regex "$IGNORE" --text --show-missing-lines --output-path "$OUT/missing.txt" >/dev/null 2>&1 || true

"$PHP" "$ROOT/scripts/coverage-check.php" "$OUT/summary.json" "$ROOT/coverage/baseline.json" ${@+"$@"}
echo "HTML report: $OUT/html/index.html"
