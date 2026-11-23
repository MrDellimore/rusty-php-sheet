//! Read every row of every sheet and report throughput.
//!
//! cargo run --release -p rusty_sheet_core --example read_all -- <file.xlsx>
//!
//! Shared strings are resolved (and their bytes counted) so the work is comparable
//! to producing PHP values.

use std::time::Instant;

use rusty_sheet_core::CellValue;
use rusty_sheet_core::xlsx::{ReadOptions, Row, XlsxReader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: read_all <file.xlsx>")?;
    let started = Instant::now();
    let mut book = XlsxReader::open(path.as_ref())?;

    let sst_started = Instant::now();
    let sst = book.shared_strings()?;
    let sst_time = sst_started.elapsed();

    let (mut rows_total, mut cells, mut string_bytes) = (0u64, 0u64, 0u64);
    for i in 0..book.sheets().len() {
        let mut rows = book.rows(i, ReadOptions::default())?;
        let mut row = Row::default();
        while rows.next_row(&mut row)? {
            rows_total += 1;
            cells += row.cells.len() as u64;
            for c in &row.cells {
                if let CellValue::SharedString(idx) = c {
                    string_bytes += rows
                        .shared_strings()
                        .get(*idx)
                        .map_or(0, |s| s.len() as u64);
                }
            }
        }
    }

    let secs = started.elapsed().as_secs_f64();
    println!(
        "rusty_sheet_core  {}\n  rows: {rows_total}  cells: {cells}  time: {secs:.2}s  rows/s: {:.0}\n  \
         shared strings: {} ({:.1} MB heap, loaded in {:.2}s)  string bytes touched: {:.0} MB",
        path,
        rows_total as f64 / secs,
        sst.len(),
        sst.heap_bytes() as f64 / 1_048_576.0,
        sst_time.as_secs_f64(),
        string_bytes as f64 / 1_048_576.0,
    );
    Ok(())
}
