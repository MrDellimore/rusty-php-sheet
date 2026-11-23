# rusty-php-sheet

A Rust-powered PHP extension for streaming large spreadsheets with low memory and CPU use.
It is intended as a drop-in accelerator for laravel sheet stream (currently OpenSpout-based).

## Layout

| Path | What |
|---|---|
| `crates/rusty_sheet_core` | Pure Rust reader/writer logic with no PHP types (`cargo test`) |
| `crates/rusty_sheet` | Thin `ext-php-rs` binding that builds the PHP extension |
| `tools/gen_fixture` | Generates the edge-case fixtures and the large synthetic benchmark file |
| `php/` | Dev harness: Pest tests, OpenSpout as the parity reference, benchmarks, IDE stubs |
| `tests/fixtures/edge` | Hand-crafted XLSX edge cases (generated; committed) |
| `tests/fixtures/laravel-excel` | Real Excel/LibreOffice/ONLYOFFICE files (MIT, see its README) |
| `tests/fixtures/generated` | Large benchmark files (gitignored; `make large-fixture`) |
| `fuzz/` | `cargo fuzz` targets (sheet, shared strings, styles, CSV, whole XLSX) |
| `coverage/` | Coverage baseline (the ratchet) and the explained exclusions |
| `scripts/` | `coverage.sh`, `fuzz.sh`, `gate.sh` |

## Prerequisites

- Rust via [rustup](https://rustup.rs) (`clippy` and `rustfmt` components)
- PHP 8.4+ with `php-config`, plus Composer
- macOS: Xcode Command Line Tools. If the build can't find `libclang`, run `brew install llvm`.
- For the regression gate: `rustup component add llvm-tools-preview`, `cargo install cargo-llvm-cov cargo-fuzz`,
  and `rustup toolchain install nightly` (cargo-fuzz needs it).

## Dev commands

```bash
make help            # list everything
make test            # Rust tests + PHP (Pest) tests with the extension loaded
make lint            # clippy (warnings are errors) + rustfmt check
make fixtures        # regenerate tests/fixtures/edge
make large-fixture   # 400k × 50 synthetic file (ROWS=100000 for a smaller one)
make bench-openspout ROWS=100000   # OpenSpout baseline

make coverage        # Rust + PHP tests on an instrumented build; fails below target/baseline or on
                     # an uncovered line not explained in coverage/exclusions.md (report: target/coverage/)
make fuzz            # every cargo-fuzz target, FUZZ_SECONDS=60 each
make bench-check     # 400k-row read/write vs php/bench/baseline.json
make gate            # all of the above + lint + sheet-stream's suite: run before finishing a phase

php php/bin/dump-openspout.php tests/fixtures/edge/dates.xlsx   # see OpenSpout's output with types
```

PhpStorm: `php/rusty_sheet.stub.php` gives autocompletion for the extension's API.
