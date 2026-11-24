# rusty_sheet

A PHP extension, written in Rust, that reads and writes XLSX and CSV files the way
[OpenSpout](https://github.com/openspout/openspout) does, hundreds of times faster, with
flat memory. It's the engine behind the `rust` reader and writer drivers of
laravel-sheet-stream, and usable on its own.

- **Reading:** XLSX, CSV and TSV, as the same PHP values OpenSpout gives (strings, ints,
  floats, bools, dates as serials, `DateTimeImmutable` or display strings).
- **Writing:** XLSX with the same styles, cell XML and parts as OpenSpout 5.15, and
  byte-identical CSV. Also borders, merged cells, autofilters, hyperlinks, data
  validation and comments (all as OpenSpout writes them), plus auto-sized columns and
  tables (as PhpSpreadsheet writes them).
- **Not supported:** `.xls` and `.ods`, and CSV in encodings other than UTF-8. These throw
  `RustySheet\UnsupportedFormatException`, so callers can fall back to OpenSpout.

## Benchmarks

400,000 rows × 50 columns, Apple Silicon, PHP 8.5, release build:

| | rusty_sheet | OpenSpout 5.15 |
|---|---|---|
| Read XLSX (131 MB, 2 M shared strings) | **3.9 s**, 69 MB RSS | about 11 min at the default `memory_limit` |
| Read CSV (172 MB) | **0.8 s** | 7.4 s |
| Write XLSX, mixed types | **7.3 s**, 30 MB RSS | 62 s, 568 MB |
| Write XLSX with auto-sized columns | 11.2 s, 30 MB RSS | — |
| Write CSV | **2.9 s** | 7.8 s |

A real 86 MB, 396k-row workbook reads in 3.4 s through laravel-sheet-stream, against
24 min 52 s with its OpenSpout driver.

## Install

PHP 8.4 or 8.5 on Linux (x86_64 and arm64, glibc and musl/Alpine) or macOS (arm64), NTS
or ZTS. With [PIE](https://github.com/php/pie):

```bash
pie install mrdellimore/rusty-sheet
```

PIE downloads the prebuilt binary for your PHP from the GitHub release and enables
it. Or download `php_rusty_sheet-{version}_php{X.Y}-{arch}-{os}-{libc}-{nts|zts}.zip`
from the [releases](https://github.com/MrDellimore/rusty-php-sheet/releases), copy
`rusty_sheet.so` to your `extension_dir`, and add `extension=rusty_sheet` to php.ini.

To build from source: Rust (stable), PHP with `php-config`, and libclang, then
`cargo build --release -p rusty_sheet`. The extension is
`target/release/librusty_sheet.so` (`.dylib` on macOS).

## Usage

```php
use RustySheet\Reader;
use RustySheet\Writer;

$reader = Reader::open('claims.xlsx', ['dates' => 'datetime']);
foreach ($reader->sheetNames() as $i => $name) {
    while (($rows = $reader->nextRows($i, 1000)) !== []) {
        foreach ($rows as $rowNumber => $cells) {
            // $cells: list of string|int|float|bool|null|DateTimeImmutable
        }
    }
}

$w = Writer::create('report.xlsx');
$w->addSheet('Claims');
$w->autoSizeColumns();
$w->setAutoFilter('A1:C1');
$w->freezePanes(1);
$header = $w->addStyle(['bold' => true, 'background' => 'D9E1F2', 'border' => ['bottom' => ['width' => 'thin']]]);
$money = $w->addStyle(['format' => '$#,##0.00_-']);
$w->addRow(['Claimant', 'Amount', 'Status'], $header);
$w->addValidation('C2:C1000', ['type' => 'list', 'values' => ['Open', 'Closed']]);
$w->addRow(['Ann', Writer::cell(1234.5, $money, 'Reviewed'), 'Open']);
$w->addRow([Writer::link('Details', "#'Detail'!A1")]);
$w->close();
```

The full API, every option and every exception is documented in
[`php/rusty_sheet.stub.php`](php/rusty_sheet.stub.php), which also gives IDEs (PhpStorm,
PHPStan) autocompletion and types.

## Differences from OpenSpout

Values and files match OpenSpout's except for these deliberate differences:

- **Reading:**
  - Shared formulas are expanded (OpenSpout returns them empty).
  - Text starting with `=` stays text.
  - Unparseable numbers become `null`.
  - Row numbers past 1,048,576 are a row error, like cells past column XFD. These throw
    `ReaderException`, or skip the row with `on_error => 'skip_row'`.
  - Dates before 1900-03-01 get their true serial (laravel-sheet-stream's own conversion
    was a day off).
- **Writing XLSX:**
  - No `<dimension>` element, except on auto-sized sheets (rows stream straight into the
    file).
  - No empty comment, VML or shared-string parts.
  - Numbers at full precision (OpenSpout keeps 14 significant digits).
  - Prompt and error texts are escaped for XML.
- **Rejected with `ValueError`:** what Excel would have to repair, namely overlapping
  merges, inline validation lists over 255 characters, and invalid or overlapping tables.
- **Beyond OpenSpout:** `hair` borders, auto-sized columns, tables and formula values in
  validation rules follow PhpSpreadsheet 5.11. Auto-sized widths match its default
  approximate mode exactly, except for formulas (not measured).

## Development

| Path | What |
|---|---|
| `crates/rusty_sheet_core` | Pure Rust reader/writer logic with no PHP types (`cargo test`) |
| `crates/rusty_sheet` | Thin `ext-php-rs` binding that builds the PHP extension |
| `tools/gen_fixture` | Generates the edge-case fixtures and the large synthetic benchmark file |
| `php/` | Dev harness: Pest tests, OpenSpout and PhpSpreadsheet as parity references, benchmarks, IDE stubs |
| `tests/fixtures/edge` | Hand-crafted XLSX edge cases (generated; committed) |
| `tests/fixtures/laravel-excel` | Real Excel/LibreOffice/ONLYOFFICE files (MIT, see its README) |
| `tests/fixtures/generated` | Large benchmark files (gitignored; `make large-fixture`) |
| `fuzz/` | `cargo fuzz` targets (sheet, shared strings, styles, CSV, whole XLSX) |
| `coverage/` | Coverage baseline (the ratchet) and the explained exclusions |
| `scripts/` | `coverage.sh`, `fuzz.sh`, `gate.sh`, `release-build.sh` |
| `.github/workflows` | CI (lint, tests, coverage), nightly fuzzing and macOS tests, and the release build for PIE |

Prerequisites: Rust via [rustup](https://rustup.rs) (`clippy`, `rustfmt`), PHP 8.4+ with
`php-config`, Composer, and on macOS the Xcode Command Line Tools (if the build can't find
`libclang`, `brew install llvm`). For the regression gate: `rustup component add
llvm-tools-preview`, `cargo install cargo-llvm-cov cargo-fuzz`, and `rustup toolchain
install nightly` (cargo-fuzz needs it).

```bash
make help            # list everything
make test            # Rust tests + PHP (Pest) tests with the extension loaded
make lint            # clippy (warnings are errors) + rustfmt check
make fixtures        # regenerate tests/fixtures/edge
make large-fixture   # 400k × 50 synthetic file (ROWS=100000 for a smaller one)

make coverage        # Rust + PHP tests on an instrumented build; fails below target/baseline or on
                     # an uncovered line not explained in coverage/exclusions.md (report: target/coverage/)
make fuzz            # every cargo-fuzz target, FUZZ_SECONDS=60 each
make bench-check     # 400k-row read/write vs php/bench/baseline.json
make gate            # all of the above + lint + sheet-stream's suite: run before finishing a phase

php php/bin/dump-openspout.php tests/fixtures/edge/dates.xlsx   # see OpenSpout's output with types
```

### Releasing

Versions follow [semantic versioning](https://semver.org). While the version is 0.x, a
patch release (`0.1.1`) only fixes bugs and a minor release (`0.2.0`) may change the API;
from `1.0.0`, minor releases add features without breaking anything. Users pin a range
(`pie install "mrdellimore/rusty-sheet:^0.1"`, or `"ext-rusty_sheet": "^0.1"` in Composer)
and get every compatible release, never a breaking one.

1. Set `version` under `[workspace.package]` in `Cargo.toml` and run `cargo check` so
   `Cargo.lock` follows. The extension reports this version (`phpversion('rusty_sheet')`).
2. Merge to `main`, then tag that commit with the same version, without a leading `v`
   (`0.1.1`), and push the tag. Never move a tag that has been released; release a new
   version instead.

The tag push runs `.github/workflows/release.yml`. It checks that the tag matches
`Cargo.toml`, builds the extension for every supported platform, PHP version and
thread-safety mode, and runs the PHP suite against each binary. Then it packages each one
under the name PIE looks for, attests them, and attaches them to a GitHub release for the
tag. Packagist picks up the tag through its GitHub webhook. `scripts/release-build.sh`
builds and packages for one platform locally.

## License

MIT
