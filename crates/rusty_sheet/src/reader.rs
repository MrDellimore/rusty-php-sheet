//! `RustySheet\Reader`: streaming spreadsheet reader for PHP (XLSX, CSV, TSV).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext_php_rs::boxed::ZBox;
use ext_php_rs::exception::PhpResult;
use ext_php_rs::flags::ClassFlags;
use ext_php_rs::prelude::*;
use ext_php_rs::types::{ZendHashTable, Zval};

use rusty_sheet_core::Error;
use rusty_sheet_core::csv::{CsvOptions, CsvReader};
use rusty_sheet_core::xlsx::{ReadOptions, Row, SharedStrings, SheetRows, Styles, XlsxReader};

use crate::convert::Converter;
use crate::exceptions::{from_core, invalid_option, reader_error, unsupported};
use crate::options::{OnError, ReaderOptions};

#[php_class]
#[php(name = "RustySheet\\Reader")]
#[php(flags = ClassFlags::Final)]
pub struct Reader {
    /// `None` once closed.
    state: Option<State>,
}

/// What was opened.
enum Book {
    Xlsx(XlsxReader),
    /// A CSV/TSV file is one sheet, named '' (as in OpenSpout).
    Csv {
        path: PathBuf,
        opts: CsvOptions,
    },
}

/// An open row stream for one sheet.
enum Stream {
    Xlsx(Box<SheetRows>),
    Csv(CsvReader),
}

struct State {
    path: String,
    book: Book,
    opts: ReaderOptions,
    /// Open sheet streams, by sheet index. Several sheets can be read interleaved.
    streams: HashMap<usize, Stream>,
    finished: HashSet<usize>,
    row: Row,
    /// Excel → PHP date formats, cached for `dates => 'formatted'`.
    php_formats: HashMap<u32, String>,
}

#[php_impl]
impl Reader {
    /// Open a spreadsheet (XLSX, CSV or TSV). The format comes from
    /// `$options['format']` or the file extension.
    ///
    /// Throws `UnsupportedFormatException` for what this extension doesn't read
    /// (callers fall back to OpenSpout), `ReaderException` for unreadable files and
    /// `ValueError` for invalid options.
    pub fn open(path: String, options: Option<&ZendHashTable>) -> PhpResult<Self> {
        let opts = ReaderOptions::parse(options)?;
        let format = opts.format.clone().unwrap_or_else(|| {
            Path::new(&path)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });
        if let Some(tz) = &opts.timezone {
            // Fail at open time rather than on the first date cell.
            validate_timezone(tz)?;
        }
        let book = match format.as_str() {
            "xlsx" => {
                Book::Xlsx(XlsxReader::open(Path::new(&path)).map_err(|e| from_core(e, &path))?)
            }
            "csv" | "tsv" => {
                let csv = csv_options(&opts, &format, &path)?;
                // Fail now for a missing or unreadable file, as for XLSX.
                std::fs::File::open(&path).map_err(|e| from_core(Error::Io(e), &path))?;
                Book::Csv {
                    path: PathBuf::from(&path),
                    opts: csv,
                }
            }
            other => {
                return Err(unsupported(format!(
                    "{path}: format '{other}' is not supported by rusty_sheet"
                )));
            }
        };
        Ok(Reader {
            state: Some(State {
                path,
                book,
                opts,
                streams: HashMap::new(),
                finished: HashSet::new(),
                row: Row::default(),
                php_formats: HashMap::new(),
            }),
        })
    }

    /// Sheet names in workbook (tab) order, without reading row data.
    /// A CSV/TSV file has one sheet, named ''.
    pub fn sheet_names(&self) -> PhpResult<Vec<String>> {
        let state = self.state()?;
        Ok(match &state.book {
            Book::Xlsx(book) => book.sheet_names().map(str::to_string).collect(),
            Book::Csv { .. } => vec![String::new()],
        })
    }

    /// The next batch of up to `$max` rows from `$sheet` (0-based index or name),
    /// keyed by 1-based source row (XLSX) or line (CSV) number. `[]` means finished.
    #[php(defaults(max = 1000))]
    pub fn next_rows(&mut self, sheet: &Zval, max: i64) -> PhpResult<ZBox<ZendHashTable>> {
        if max < 1 {
            return Err(invalid_option("max", "must be at least 1"));
        }
        let state = self.state_mut()?;
        let index = state.resolve_sheet(sheet)?;
        let mut out = ZendHashTable::with_capacity(max.min(10_000) as u32);
        if state.finished.contains(&index) {
            return Ok(out);
        }

        if !state.streams.contains_key(&index) {
            let stream = state.open_stream(index)?;
            state.streams.insert(index, stream);
        }

        let sheet_name = match &state.book {
            Book::Xlsx(book) => book
                .sheet_names()
                .nth(index)
                .unwrap_or_default()
                .to_string(),
            Book::Csv { .. } => String::new(),
        };
        let State {
            path,
            opts,
            streams,
            finished,
            row,
            php_formats,
            ..
        } = state;
        let stream = streams.get_mut(&index).expect("inserted above");
        // The converter borrows the shared strings and styles through the stream's
        // Arcs; CSV has neither.
        let (shared, styles) = match stream {
            Stream::Xlsx(rows) => (
                Arc::clone(rows.shared_strings_arc()),
                Arc::clone(rows.styles_arc()),
            ),
            Stream::Csv(_) => (
                Arc::new(SharedStrings::default()),
                Arc::new(Styles::default()),
            ),
        };
        let mut conv = Converter::new(opts, path, &sheet_name, &shared, &styles, php_formats);

        let mut count = 0;
        while count < max {
            match stream.next_row(row) {
                Ok(true) => {
                    let arr = match conv.row(row.number, &row.cells) {
                        Ok(arr) => arr,
                        // Don't let a retry carry on past the row that failed.
                        Err(e) => {
                            streams.remove(&index);
                            finished.insert(index);
                            return Err(e);
                        }
                    };
                    out.insert_at_index(i64::from(row.number), arr)?;
                    count += 1;
                }
                Ok(false) => {
                    streams.remove(&index);
                    finished.insert(index);
                    break;
                }
                // The parser has already moved past the bad row; carry on.
                Err(Error::InvalidData { .. }) if opts.on_error == OnError::SkipRow => {}
                Err(e) => {
                    streams.remove(&index);
                    finished.insert(index);
                    return Err(from_core(e, path));
                }
            }
        }
        Ok(out)
    }

    /// How many rows `nextRows($sheet)` yields in total, under the same options
    /// (`preserve_empty_rows`, `on_error`), without converting any cell to PHP.
    /// Reads the sheet on a stream of its own: a `nextRows()` in progress is not
    /// affected. Cell values aren't checked, so a sheet whose rows `nextRows()`
    /// would throw on (`formulas_without_cached_value => 'throw'`) still counts.
    pub fn row_count(&mut self, sheet: &Zval) -> PhpResult<i64> {
        let state = self.state_mut()?;
        let index = state.resolve_sheet(sheet)?;
        let mut stream = state.open_stream(index)?;
        let mut row = Row::default();
        let mut count = 0;
        loop {
            match stream.next_row(&mut row) {
                Ok(true) => count += 1,
                Ok(false) => return Ok(count),
                Err(Error::InvalidData { .. }) if state.opts.on_error == OnError::SkipRow => {}
                Err(e) => return Err(from_core(e, &state.path)),
            }
        }
    }

    /// Release the file handles. Further calls throw.
    pub fn close(&mut self) {
        self.state = None;
    }
}

impl Reader {
    fn state(&self) -> PhpResult<&State> {
        self.state
            .as_ref()
            .ok_or_else(|| reader_error("the reader is closed"))
    }

    fn state_mut(&mut self) -> PhpResult<&mut State> {
        self.state
            .as_mut()
            .ok_or_else(|| reader_error("the reader is closed"))
    }
}

impl Stream {
    fn next_row(&mut self, row: &mut Row) -> rusty_sheet_core::Result<bool> {
        match self {
            Stream::Xlsx(rows) => rows.next_row(row),
            Stream::Csv(csv) => csv.next_row(row),
        }
    }
}

impl State {
    /// A fresh row stream for sheet `index`, from the first row.
    fn open_stream(&mut self, index: usize) -> PhpResult<Stream> {
        Ok(match &mut self.book {
            Book::Xlsx(book) => {
                let opts = ReadOptions {
                    preserve_empty_rows: self.opts.preserve_empty_rows,
                };
                Stream::Xlsx(Box::new(
                    book.rows(index, opts)
                        .map_err(|e| from_core(e, &self.path))?,
                ))
            }
            Book::Csv { path, opts } => {
                Stream::Csv(CsvReader::open(path, *opts).map_err(|e| from_core(e, &self.path))?)
            }
        })
    }

    fn resolve_sheet(&self, sheet: &Zval) -> PhpResult<usize> {
        let sheet = sheet.dereference();
        let count = match &self.book {
            Book::Xlsx(book) => book.sheets().len(),
            Book::Csv { .. } => 1,
        };
        if let Some(i) = sheet.long().filter(|_| sheet.is_long()) {
            return usize::try_from(i)
                .ok()
                .filter(|&i| i < count)
                .ok_or_else(|| {
                    reader_error(format!(
                        "{}: no sheet at index {i} (the workbook has {count})",
                        self.path
                    ))
                });
        }
        if let Some(name) = sheet.str().filter(|_| sheet.is_string()) {
            let found = match &self.book {
                Book::Xlsx(book) => book.sheet_index(name),
                Book::Csv { .. } => name.is_empty().then_some(0),
            };
            return found
                .ok_or_else(|| reader_error(format!("{}: no sheet named '{name}'", self.path)));
        }
        Err(invalid_option(
            "sheet",
            "expected a sheet index (int) or name (string)",
        ))
    }
}

/// CSV settings for a format: TSV defaults to a tab delimiter. Only UTF-8 is read
/// here; other encodings are reported as unsupported so callers fall back to
/// OpenSpout, whose per-cell iconv conversion we don't replicate.
fn csv_options(opts: &ReaderOptions, format: &str, path: &str) -> PhpResult<CsvOptions> {
    if let Some(enc) = &opts.csv.encoding
        && !matches!(enc.to_ascii_lowercase().as_str(), "utf-8" | "utf8")
    {
        return Err(unsupported(format!(
            "{path}: CSV encoding '{enc}' is not supported by rusty_sheet (only UTF-8)"
        )));
    }
    Ok(CsvOptions {
        delimiter: opts
            .csv
            .delimiter
            .unwrap_or(if format == "tsv" { b'\t' } else { b',' }),
        enclosure: opts.csv.enclosure.unwrap_or(b'"'),
        escape: opts.csv.escape.unwrap_or(None),
        preserve_empty_rows: opts.preserve_empty_rows,
    })
}

fn validate_timezone(tz: &str) -> PhpResult<()> {
    let valid = ext_php_rs::zend::Function::try_from_function("timezone_identifiers_list")
        .and_then(|f| f.try_call(vec![]).ok())
        .and_then(|list| {
            list.array()
                .map(|a| a.values().any(|v| v.str() == Some(tz)))
        })
        .unwrap_or(true);
    // Also accept offsets/abbreviations PHP understands ("+02:00", "UTC", "EST").
    if valid || tz.eq_ignore_ascii_case("UTC") || tz.starts_with(['+', '-']) {
        Ok(())
    } else {
        Err(invalid_option(
            "timezone",
            &format!("unknown timezone '{tz}'"),
        ))
    }
}
