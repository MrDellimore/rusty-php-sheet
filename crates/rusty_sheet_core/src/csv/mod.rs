//! Streaming CSV/TSV reader with OpenSpout's semantics (`Reader\CSV\RowIterator`):
//! records are parsed by a port of PHP's `fgetcsv()`, a UTF-8 BOM is skipped, blank
//! lines are skipped unless `preserve_empty_rows`, every value is a string, and rows
//! are **not** trimmed (sheet-stream trims XLSX rows only).
//!
//! Only UTF-8 input is handled here. OpenSpout converts other encodings per cell
//! with iconv (with special trimming for UTF-16/32); callers fall back to it for those.

mod fgetcsv;

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use fgetcsv::{LineSource, parse_record};

use crate::value::CellValue;
use crate::xlsx::Row;
use crate::{Error, Result};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Debug, Clone, Copy)]
pub struct CsvOptions {
    pub delimiter: u8,
    pub enclosure: u8,
    /// `None` = no escape character (OpenSpout's default, `FIELD_ESCAPE = ''`).
    pub escape: Option<u8>,
    pub preserve_empty_rows: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        CsvOptions {
            delimiter: b',',
            enclosure: b'"',
            escape: None,
            preserve_empty_rows: false,
        }
    }
}

pub struct CsvReader {
    src: LineSource<BufReader<Box<dyn Read + Send>>>,
    opts: CsvOptions,
}

impl CsvReader {
    pub fn open(path: &Path, opts: CsvOptions) -> Result<Self> {
        Self::new(Box::new(File::open(path)?), opts)
    }

    pub fn from_bytes(bytes: Vec<u8>, opts: CsvOptions) -> Result<Self> {
        Self::new(Box::new(std::io::Cursor::new(bytes)), opts)
    }

    fn new(inner: Box<dyn Read + Send>, opts: CsvOptions) -> Result<Self> {
        let mut reader = BufReader::with_capacity(128 * 1024, inner);
        if reader.fill_buf()?.starts_with(UTF8_BOM) {
            reader.consume(UTF8_BOM.len());
        }
        Ok(CsvReader {
            src: LineSource::new(reader),
            opts,
        })
    }

    /// Read the next record into `out`. `out.number` is the 1-based line the record
    /// starts on. Returns `false` at the end.
    pub fn next_row(&mut self, out: &mut Row) -> Result<bool> {
        loop {
            let Some(line) = self.src.next_line()? else {
                return Ok(false);
            };
            let start = u32::try_from(self.src.line).map_err(|_| Error::InvalidData {
                part: "csv".into(),
                message: "more than 4 billion lines".into(),
            })?;
            let record = parse_record(
                line,
                &mut self.src,
                self.opts.delimiter,
                self.opts.enclosure,
                self.opts.escape,
            )?;
            out.number = start;
            out.cells.clear();
            match record {
                // A blank line: fgetcsv's [null], which OpenSpout turns into [''].
                None if self.opts.preserve_empty_rows => {
                    out.cells.push(CellValue::Empty);
                    return Ok(true);
                }
                None => continue,
                Some(fields) => {
                    out.cells.extend(fields.into_iter().map(field_value));
                    return Ok(true);
                }
            }
        }
    }
}

/// PHP strings are bytes: valid UTF-8 becomes a `String`, anything else is kept as is.
fn field_value(bytes: Vec<u8>) -> CellValue {
    if bytes.is_empty() {
        return CellValue::Empty;
    }
    match String::from_utf8(bytes) {
        Ok(s) => CellValue::String(s),
        Err(e) => CellValue::Bytes(e.into_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(input: &[u8], opts: CsvOptions) -> Vec<(u32, Vec<CellValue>)> {
        let mut r = CsvReader::from_bytes(input.to_vec(), opts).unwrap();
        let mut row = Row::default();
        let mut out = vec![];
        while r.next_row(&mut row).unwrap() {
            out.push((row.number, row.cells.clone()));
        }
        out
    }

    fn s(v: &str) -> CellValue {
        CellValue::String(v.into())
    }

    #[test]
    fn skips_bom_and_blank_lines() {
        let rows = read(b"\xEF\xBB\xBFa,b\n\n\nc,\n", CsvOptions::default());
        assert_eq!(
            rows,
            [
                (1, vec![s("a"), s("b")]),
                (4, vec![s("c"), CellValue::Empty]),
            ]
        );
    }

    #[test]
    fn preserves_blank_lines_as_single_empty_cell() {
        let opts = CsvOptions {
            preserve_empty_rows: true,
            ..Default::default()
        };
        let rows = read(b"a\n\nb\n", opts);
        assert_eq!(
            rows,
            [
                (1, vec![s("a")]),
                (2, vec![CellValue::Empty]),
                (3, vec![s("b")]),
            ]
        );
    }

    #[test]
    fn custom_delimiter_and_line_numbers_for_multiline_records() {
        let opts = CsvOptions {
            delimiter: b';',
            ..Default::default()
        };
        let rows = read(b"\"x\ny\";2\nz;3", opts);
        assert_eq!(
            rows,
            [(1, vec![s("x\ny"), s("2")]), (3, vec![s("z"), s("3")])]
        );
    }

    #[test]
    fn invalid_utf8_passes_through() {
        let rows = read(b"caf\xE9\n", CsvOptions::default());
        assert_eq!(rows, [(1, vec![CellValue::Bytes(b"caf\xE9".to_vec())])]);
    }
}
