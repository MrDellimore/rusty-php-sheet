//! Criterion benchmarks for the XLSX reader.
//!
//! Needs the synthetic fixture: `make large-fixture ROWS=10000` (the bench is
//! skipped with a hint if it's missing). For the full 400k-row number use
//! `make read-bench`, which reports rows/s and peak memory.

use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use rusty_sheet_core::xlsx::{ReadOptions, Row, XlsxReader};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(rel)
}

/// Read all rows of all sheets; returns the row count.
fn read_all(path: &Path) -> u64 {
    let mut book = XlsxReader::open(path).unwrap();
    let mut n = 0;
    for i in 0..book.sheets().len() {
        let mut rows = book.rows(i, ReadOptions::default()).unwrap();
        let mut row = Row::default();
        while rows.next_row(&mut row).unwrap() {
            black_box(&row);
            n += 1;
        }
    }
    n
}

fn bench_read(c: &mut Criterion) {
    let large = fixture("generated/large-10000.xlsx");
    if large.exists() {
        let rows = read_all(&large);
        let mut group = c.benchmark_group("xlsx");
        group.sample_size(20);
        group.throughput(Throughput::Elements(rows));
        group.bench_function("read 10k rows x 50 cols", |b| b.iter(|| read_all(&large)));
        group.finish();
    } else {
        eprintln!("skipping large bench: run `make large-fixture ROWS=10000` first");
    }

    let small = fixture("edge/dates.xlsx");
    c.bench_function("xlsx/open + read dates.xlsx", |b| {
        b.iter(|| read_all(&small))
    });
}

criterion_group!(benches, bench_read);
criterion_main!(benches);
