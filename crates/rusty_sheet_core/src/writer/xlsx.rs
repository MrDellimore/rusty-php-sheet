//! Streaming XLSX writer producing the same XML as OpenSpout's XLSX writer.
//!
//! Rows go straight into the zip, one sheet after another, so memory stays flat
//! and nothing is buffered on disk. The differences from OpenSpout are deliberate:
//! - no `<dimension>` (it would need the whole sheet buffered first; it's optional);
//! - no empty comments / VML / shared-strings parts (OpenSpout always writes them);
//! - numbers are written with full precision; OpenSpout goes through PHP's
//!   `precision` ini setting and keeps 14 significant digits (`1/3` → `0.33333333333333`).
//!
//! Merged cells, the autofilter and hyperlinks are written after `<sheetData>` when a
//! sheet ends, as OpenSpout does; only the links are kept in memory until then.
//!
//! A sheet with auto-sized columns can't stream: `<cols>` comes before the rows. Its
//! rows go to a temp file instead, and the sheet is written at its end, with the
//! measured widths and (because the size is then known) a `<dimension>`.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::autosize::{self, Measure};
use super::escape::{escape, escape_into, truncate_for_cell};
use super::style::{Style, StyleRegistry};
use crate::{Error, Result};

const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A value to write. Strings are written as given (no formula detection here).
#[derive(Debug, Clone, Copy)]
pub enum CellData<'a> {
    /// Not written unless its style has a fill or a number format.
    Empty,
    Str(&'a str),
    /// Formula text without the leading `=`.
    Formula(&'a str),
    Int(i64),
    /// Also used for date serials and durations.
    Float(f64),
    Bool(bool),
}

#[derive(Debug, Clone, Copy)]
pub struct Cell<'a> {
    pub data: CellData<'a>,
    /// A handle from [`XlsxWriter::add_style`]; `None` uses style 0.
    pub style: Option<usize>,
    /// Treat the string as multiline (wrap text) even if `data` has no "\n". For
    /// callers whose original value had a line break but is written differently
    /// (OpenSpout writes invalid UTF-8 as '' yet still checks the raw bytes).
    pub multiline: bool,
    /// A hyperlink. Only string cells can carry one (OpenSpout: `StringCell`).
    pub link: Option<Link<'a>>,
}

/// A hyperlink on a cell.
#[derive(Debug, Clone, Copy)]
pub struct Link<'a> {
    /// A URL; `#Sheet!A1` or `#'My sheet'!A1` for a place in the workbook; or
    /// PhpSpreadsheet's `sheet://'My sheet'!A1`, which means the same as `#…`.
    pub target: &'a str,
    pub tooltip: Option<&'a str>,
}

/// A rectangle of cells: 0-based columns, 1-based rows, first ≤ last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRange {
    pub first_col: u32,
    pub first_row: u32,
    pub last_col: u32,
    pub last_row: u32,
}

impl CellRange {
    /// `A1:C3`, `$A$1:$C$3`, `c3:a1` (corners in any order) or a single cell `B2`.
    pub fn parse(range: &str) -> Option<CellRange> {
        let (a, b) = range.split_once(':').unwrap_or((range, range));
        let (c1, r1) = parse_cell_ref(a)?;
        let (c2, r2) = parse_cell_ref(b)?;
        Some(CellRange {
            first_col: c1.min(c2),
            first_row: r1.min(r2),
            last_col: c1.max(c2),
            last_row: r1.max(r2),
        })
    }

    pub fn is_single_cell(&self) -> bool {
        self.first_col == self.last_col && self.first_row == self.last_row
    }

    pub fn overlaps(&self, other: &CellRange) -> bool {
        self.first_col <= other.last_col
            && other.first_col <= self.last_col
            && self.first_row <= other.last_row
            && other.first_row <= self.last_row
    }

    /// `$A$1:$C$3`, as in a defined name.
    fn absolute(&self) -> String {
        let mut s = String::new();
        for (col, row, sep) in [
            (self.first_col, self.first_row, ":"),
            (self.last_col, self.last_row, ""),
        ] {
            s.push('$');
            column_letters(col as usize, &mut s);
            let _ = write!(s, "${row}{sep}");
        }
        s
    }
}

impl std::fmt::Display for CellRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        column_letters(self.first_col as usize, &mut s);
        let _ = write!(s, "{}:", self.first_row);
        column_letters(self.last_col as usize, &mut s);
        let _ = write!(s, "{}", self.last_row);
        f.write_str(&s)
    }
}

/// `B12` or `$B$12` → (1, 12). Columns A..XFD, rows 1..1048576.
fn parse_cell_ref(r: &str) -> Option<(u32, u32)> {
    let r = r.trim().strip_prefix('$').unwrap_or(r.trim());
    let letters = r.bytes().take_while(u8::is_ascii_alphabetic).count();
    if !(1..=3).contains(&letters) {
        return None;
    }
    let col = r[..letters].bytes().fold(0u32, |acc, b| {
        acc * 26 + u32::from(b.to_ascii_uppercase() - b'A' + 1)
    }) - 1;
    let digits = r[letters..].strip_prefix('$').unwrap_or(&r[letters..]);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    (col < MAX_COLUMNS as u32 && (1..=MAX_ROWS).contains(&row)).then_some((col, row))
}

/// Excel's sheet limits.
pub const MAX_COLUMNS: usize = 16_384;
pub const MAX_ROWS: u32 = 1_048_576;

#[derive(Debug, Clone, Default)]
struct SheetView {
    show_grid_lines: bool,
    /// Rows frozen at the top.
    freeze_rows: u32,
    /// Columns frozen on the left.
    freeze_columns: u32,
}

struct Sheet {
    name: String,
    view: Option<SheetView>,
    /// `(first column, last column, width)`, 1-based, in call order.
    widths: Vec<(u32, u32, f64)>,
    /// Rows started (a row index is used even when an empty row is skipped).
    rows: u32,
    /// The sheet's zip entry has been started (header written).
    open: bool,
    auto_filter: Option<CellRange>,
    /// In call order (OpenSpout writes them in that order).
    merges: Vec<CellRange>,
    /// `<hyperlink …/>` elements, in cell order.
    hyperlinks: String,
    /// The sheet's `.rels` entries for links to URLs.
    link_rels: String,
    external_links: u32,
    auto_size: Option<AutoSize>,
}

/// Auto-sized columns of a sheet and what has been measured so far.
struct AutoSize {
    /// 0-based columns to size; `None` sizes every column.
    columns: Option<Vec<bool>>,
    /// Widest cell per 0-based column (PhpSpreadsheet units), `None` if no cell yet.
    widths: Vec<Option<f64>>,
    /// The most cells in a written row (for `<dimension>`).
    max_cells: usize,
    rows: Option<TempRows>,
}

impl AutoSize {
    fn sizes(&self, col: usize) -> bool {
        self.columns
            .as_ref()
            .is_none_or(|c| c.get(col).copied().unwrap_or(false))
    }
}

/// The rows of a buffered sheet, in a temp file removed when dropped (also on
/// errors and when the writer is dropped without `close()`).
struct TempRows {
    path: PathBuf,
    file: Option<BufWriter<File>>,
}

impl TempRows {
    fn create(dir: &Path) -> Result<Self> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("rusty_sheet_{}_{n}.rows.xml", std::process::id()));
        let file = File::options().write(true).create_new(true).open(&path)?;
        Ok(TempRows {
            path,
            file: Some(BufWriter::with_capacity(256 * 1024, file)),
        })
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.file
            .as_mut()
            .expect("written before it is copied")
            .write_all(bytes)?;
        Ok(())
    }

    /// Copy everything written into `out`.
    fn copy_into(&mut self, out: &mut impl Write) -> Result<()> {
        self.file
            .take()
            .expect("copied once, at the end of the sheet")
            .flush()?;
        let mut file = std::io::BufReader::with_capacity(256 * 1024, File::open(&self.path)?);
        std::io::copy(&mut file, out)?;
        Ok(())
    }
}

impl Drop for TempRows {
    fn drop(&mut self) {
        self.file = None;
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct XlsxWriter {
    /// `None` once the file is finished.
    zip: Option<ZipWriter<BufWriter<File>>>,
    opts: SimpleFileOptions,
    sheets: Vec<Sheet>,
    /// Whether the first `add_sheet` call has named the initial sheet.
    named_first: bool,
    styles: Vec<Style>,
    registry: StyleRegistry,
    buf: String,
    /// Set once the workbook parts are written; `Drop` finishes the file otherwise.
    closed: bool,
    /// Where auto-sized sheets buffer their rows.
    temp_dir: PathBuf,
}

impl XlsxWriter {
    pub fn create(path: &Path) -> Result<Self> {
        let file = BufWriter::with_capacity(256 * 1024, File::create(path)?);
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        Ok(XlsxWriter {
            zip: Some(ZipWriter::new(file)),
            opts,
            sheets: vec![Sheet::new("Sheet1".into())],
            named_first: false,
            styles: Vec::new(),
            registry: StyleRegistry::default(),
            buf: String::with_capacity(16 * 1024),
            closed: false,
            temp_dir: std::env::temp_dir(),
        })
    }

    /// Where auto-sized sheets buffer their rows (default: the system temp dir).
    pub fn set_temp_dir(&mut self, dir: &Path) {
        self.temp_dir = dir.to_path_buf();
    }

    /// Size these 1-based columns (all, for `None`) to their content, like
    /// PhpSpreadsheet's auto-size (approximate mode). Before the sheet's first row.
    /// `set_column_width` wins for the columns it names.
    pub fn auto_size_columns(&mut self, columns: Option<&[u32]>) -> Result<()> {
        let columns = match columns {
            None => None,
            Some(cols) => {
                let mut set = Vec::new();
                for &c in cols {
                    if c == 0 || c as usize > MAX_COLUMNS {
                        return Err(Error::InvalidArgument(format!(
                            "column {c} is out of range: columns are 1-based, up to {MAX_COLUMNS}"
                        )));
                    }
                    let i = c as usize - 1;
                    if set.len() <= i {
                        set.resize(i + 1, false);
                    }
                    set[i] = true;
                }
                Some(set)
            }
        };
        self.current_before_rows("auto-size columns")?.auto_size = Some(AutoSize {
            columns,
            widths: Vec::new(),
            max_cells: 0,
            rows: None,
        });
        Ok(())
    }

    /// Register a style; the returned handle is used in [`Cell::style`].
    /// Like OpenSpout (object identity), every call is a distinct style.
    pub fn add_style(&mut self, style: Style) -> usize {
        self.styles.push(style);
        self.styles.len() - 1
    }

    /// The first call names the initial sheet; later calls start a new sheet.
    pub fn add_sheet(&mut self, name: Option<&str>) -> Result<()> {
        if !self.named_first {
            self.named_first = true;
            if let Some(name) = name {
                self.validate_name(name, 0)?;
                self.sheets[0].name = name.to_string();
            }
            return Ok(());
        }
        let index = self.sheets.len();
        let name = match name {
            Some(n) => {
                self.validate_name(n, index)?;
                n.to_string()
            }
            None => {
                // OpenSpout validates default names too (a user may already have taken it).
                let n = format!("Sheet{}", index + 1);
                self.validate_name(&n, index)?;
                n
            }
        };
        self.finish_current_sheet()?;
        self.sheets.push(Sheet::new(name));
        Ok(())
    }

    /// Set the width of 1-based columns; consecutive columns become one range.
    /// Must be called before the sheet's first row.
    pub fn set_column_width(&mut self, width: f64, columns: &[u32]) -> Result<()> {
        let sheet = self.current_before_rows("set column widths")?;
        let mut start: Option<(u32, u32)> = None;
        if let Some(&c) = columns
            .iter()
            .find(|&&c| c == 0 || c as usize > MAX_COLUMNS)
        {
            return Err(invalid(&format!(
                "column {c} is out of range: columns are 1-based, up to {MAX_COLUMNS}"
            )));
        }
        for &c in columns {
            start = match start {
                Some((s, prev)) if c == prev + 1 => Some((s, c)),
                Some((s, prev)) => {
                    sheet.widths.push((s, prev, width));
                    Some((c, c))
                }
                None => Some((c, c)),
            };
        }
        if let Some((s, e)) = start {
            sheet.widths.push((s, e, width));
        }
        Ok(())
    }

    pub fn freeze(&mut self, rows: u32, columns: u32) -> Result<()> {
        if rows >= MAX_ROWS || columns as usize >= MAX_COLUMNS {
            return Err(invalid(&format!(
                "can't freeze {rows} rows and {columns} columns: a sheet has {MAX_ROWS} rows and {MAX_COLUMNS} columns"
            )));
        }
        let view = self.view_before_rows("freeze panes")?;
        view.freeze_rows = rows;
        view.freeze_columns = columns;
        Ok(())
    }

    pub fn show_grid_lines(&mut self, show: bool) -> Result<()> {
        self.view_before_rows("change gridlines")?.show_grid_lines = show;
        Ok(())
    }

    /// Merge a range of the current sheet (`A1:H1`), any time before the sheet ends.
    /// Overlapping merges and single cells are rejected (Excel would repair the file).
    pub fn merge_cells(&mut self, range: &str) -> Result<()> {
        let r = parse_range_arg(range)?;
        if r.is_single_cell() {
            return Err(Error::InvalidArgument(format!(
                "can't merge {range}: a merge needs at least two cells"
            )));
        }
        let sheet = self.sheets.last_mut().expect("always one sheet");
        if let Some(other) = sheet.merges.iter().find(|m| m.overlaps(&r)) {
            return Err(Error::InvalidArgument(format!(
                "can't merge {r}: it overlaps the merged range {other}"
            )));
        }
        sheet.merges.push(r);
        Ok(())
    }

    /// Put an autofilter on a range of the current sheet. Before its first row:
    /// the sheet header changes too (OpenSpout's `<sheetPr filterMode>`).
    pub fn set_auto_filter(&mut self, range: &str) -> Result<()> {
        let r = parse_range_arg(range)?;
        self.current_before_rows("set an autofilter")?.auto_filter = Some(r);
        Ok(())
    }

    /// Write a row. A row whose cells are all empty is skipped, but still uses up a
    /// row number (OpenSpout parity).
    pub fn add_row(&mut self, cells: &[Cell<'_>], height: Option<f64>) -> Result<()> {
        // Validate everything first, so a failed call leaves no trace.
        if cells.len() > MAX_COLUMNS {
            return Err(invalid(&format!(
                "a row can't have more than {MAX_COLUMNS} cells (got {})",
                cells.len()
            )));
        }
        if let Some(h) = cells
            .iter()
            .filter_map(|c| c.style)
            .find(|&h| h >= self.styles.len())
        {
            return Err(invalid(&format!("unknown style handle {h}")));
        }
        if cells
            .iter()
            .any(|c| c.link.is_some() && !matches!(c.data, CellData::Str(_)))
        {
            return Err(Error::InvalidArgument(
                "only text cells can carry a hyperlink".into(),
            ));
        }
        // OpenSpout (SHOULD_CREATE_NEW_SHEETS_AUTOMATICALLY) continues on a new sheet.
        if self.sheets.last().is_some_and(|s| s.rows >= MAX_ROWS) {
            self.named_first = true;
            self.add_sheet(None)?;
        }
        self.ensure_sheet_open()?;
        let row_number = {
            let sheet = self.sheets.last_mut().expect("always one sheet");
            sheet.rows += 1;
            sheet.rows
        };
        if cells.iter().all(|c| matches!(c.data, CellData::Empty)) {
            return Ok(());
        }

        self.buf.clear();
        let _ = write!(
            self.buf,
            "<row r=\"{row_number}\" spans=\"1:{}\"",
            cells.len()
        );
        if let Some(h) = height.filter(|h| *h > 0.0) {
            let _ = write!(self.buf, " ht=\"{}\" customHeight=\"1\"", number(h));
        }
        self.buf.push('>');

        let mut col_letters = String::with_capacity(4);
        for (col, cell) in cells.iter().enumerate() {
            let style_id = match cell.style {
                Some(h) => self.registry.id_for(h, &self.styles[h]),
                None => 0,
            };
            let style_id = match cell.data {
                CellData::Str(s) if cell.multiline || s.contains('\n') => {
                    self.registry.wrap_text_id(style_id)
                }
                _ => style_id,
            };
            if matches!(cell.data, CellData::Empty)
                && !self.registry.applies_to_empty_cell(style_id)
            {
                continue;
            }
            let sheet = self.sheets.last_mut().expect("always one sheet");
            if let Some(auto) = sheet.auto_size.as_mut().filter(|a| a.sizes(col))
                && let Some(width) = measure(
                    &cell.data,
                    self.registry.style(style_id),
                    col as u32,
                    row_number,
                    sheet.auto_filter.as_ref(),
                    &sheet.merges,
                )
            {
                if auto.widths.len() <= col {
                    auto.widths.resize(col + 1, None);
                }
                let w = &mut auto.widths[col];
                *w = Some(w.map_or(width, |old| old.max(width)));
            }
            col_letters.clear();
            column_letters(col, &mut col_letters);
            let _ = write!(
                self.buf,
                "<c r=\"{col_letters}{row_number}\" s=\"{style_id}\""
            );
            if let Some(link) = cell.link {
                let sheet = self.sheets.last_mut().expect("always one sheet");
                let cell_ref = format!("{col_letters}{row_number}");
                sheet.add_link(&cell_ref, link);
            }
            match cell.data {
                CellData::Empty => self.buf.push_str("/>"),
                CellData::Str(s) => {
                    self.buf.push_str(" t=\"inlineStr\"><is><t>");
                    escape_into(truncate_for_cell(s), &mut self.buf);
                    self.buf.push_str("</t></is></c>");
                }
                CellData::Formula(f) => {
                    self.buf.push_str("><f>");
                    escape_into(f, &mut self.buf);
                    self.buf.push_str("</f></c>");
                }
                CellData::Int(i) => {
                    let _ = write!(self.buf, "><v>{i}</v></c>");
                }
                CellData::Float(f) => {
                    let _ = write!(self.buf, "><v>{}</v></c>", number(f));
                }
                CellData::Bool(b) => {
                    let _ = write!(self.buf, " t=\"b\"><v>{}</v></c>", u8::from(b));
                }
            }
        }
        self.buf.push_str("</row>");
        let sheet = self.sheets.last_mut().expect("always one sheet");
        if let Some(auto) = sheet.auto_size.as_mut() {
            auto.max_cells = auto.max_cells.max(cells.len());
            auto.rows
                .as_mut()
                .expect("opened with the sheet")
                .write(self.buf.as_bytes())?;
            return Ok(());
        }
        let zip = self
            .zip
            .as_mut()
            .expect("writes happen before the file is finished");
        zip.write_all(self.buf.as_bytes())?;
        Ok(())
    }

    /// Finish the file. Without this the file is incomplete.
    pub fn close(mut self) -> Result<()> {
        self.finish()
    }

    /// Write the remaining parts and the zip directory. Idempotent.
    fn finish(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.finish_current_sheet()?;
        // Sheets that never received a row still need a part.
        for i in 0..self.sheets.len() {
            if !self.sheets[i].open {
                self.start_sheet_part(i)?;
                self.sheets[i].open = true;
                self.end_sheet_part(i)?;
            }
        }
        let parts = [
            ("[Content_Types].xml", self.content_types()),
            ("_rels/.rels", ROOT_RELS.to_string()),
            ("docProps/app.xml", APP_XML.to_string()),
            ("docProps/core.xml", core_xml()),
            ("xl/workbook.xml", self.workbook_xml()),
            ("xl/_rels/workbook.xml.rels", self.workbook_rels()),
            ("xl/styles.xml", self.registry.styles_xml()),
        ];
        for (name, content) in parts {
            let opts = self.opts;
            self.zip().start_file(name, opts)?;
            self.zip().write_all(content.as_bytes())?;
        }
        let zip = self.zip.take().expect("finished only once");
        zip.finish()?.flush()?;
        Ok(())
    }

    fn zip(&mut self) -> &mut ZipWriter<BufWriter<File>> {
        self.zip
            .as_mut()
            .expect("writes happen before the file is finished")
    }

    fn current_before_rows(&mut self, what: &str) -> Result<&mut Sheet> {
        let sheet = self.sheets.last_mut().expect("always one sheet");
        if sheet.open {
            return Err(invalid(&format!(
                "can't {what} after the sheet's first row has been written"
            )));
        }
        Ok(sheet)
    }

    fn view_before_rows(&mut self, what: &str) -> Result<&mut SheetView> {
        let sheet = self.current_before_rows(what)?;
        Ok(sheet.view.get_or_insert_with(|| SheetView {
            show_grid_lines: true,
            ..SheetView::default()
        }))
    }

    fn ensure_sheet_open(&mut self) -> Result<()> {
        let i = self.sheets.len() - 1;
        if !self.sheets[i].open {
            // An auto-sized sheet starts in a temp file; its zip entry comes at the end.
            if let Some(auto) = self.sheets[i].auto_size.as_mut() {
                auto.rows = Some(TempRows::create(&self.temp_dir)?);
            } else {
                self.start_sheet_part(i)?;
            }
            self.sheets[i].open = true;
        }
        Ok(())
    }

    fn finish_current_sheet(&mut self) -> Result<()> {
        if self.sheets.last().is_some_and(|s| s.open) {
            self.end_sheet_part(self.sheets.len() - 1)?;
        }
        Ok(())
    }

    /// Everything after the rows (OpenSpout's order: autofilter, merges, hyperlinks),
    /// then the sheet's `.rels` part if it links to URLs.
    fn end_sheet_part(&mut self, index: usize) -> Result<()> {
        if let Some(mut rows) = self.sheets[index]
            .auto_size
            .as_mut()
            .and_then(|a| a.rows.take())
        {
            self.start_sheet_part(index)?;
            let zip = self
                .zip
                .as_mut()
                .expect("writes happen before the file is finished");
            rows.copy_into(zip)?;
        }
        let sheet = &mut self.sheets[index];
        let mut s = String::from("</sheetData>");
        if let Some(f) = sheet.auto_filter {
            let _ = write!(s, "<autoFilter ref=\"{f}\"/>");
        }
        if !sheet.merges.is_empty() {
            let _ = write!(s, "<mergeCells count=\"{}\">", sheet.merges.len());
            for m in std::mem::take(&mut sheet.merges) {
                let _ = write!(s, "<mergeCell ref=\"{m}\"/>");
            }
            s.push_str("</mergeCells>");
        }
        if !sheet.hyperlinks.is_empty() {
            s.push_str("<hyperlinks>");
            s.push_str(&std::mem::take(&mut sheet.hyperlinks));
            s.push_str("</hyperlinks>");
        }
        s.push_str("</worksheet>");
        let rels = std::mem::take(&mut sheet.link_rels);
        self.zip().write_all(s.as_bytes())?;
        if !rels.is_empty() {
            let opts = self.opts;
            self.zip().start_file(
                format!("xl/worksheets/_rels/sheet{}.xml.rels", index + 1),
                opts,
            )?;
            let part = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
                 <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n\
                 {rels}</Relationships>"
            );
            self.zip().write_all(part.as_bytes())?;
        }
        Ok(())
    }

    /// Start `xl/worksheets/sheet{n}.xml` and write everything before the rows.
    fn start_sheet_part(&mut self, index: usize) -> Result<()> {
        let opts = self.opts;
        self.zip()
            .start_file(format!("xl/worksheets/sheet{}.xml", index + 1), opts)?;
        let sheet = &self.sheets[index];
        let mut s = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <worksheet xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\">"
        );
        // OpenSpout always writes fitToPage along with an autofilter.
        if sheet.auto_filter.is_some() {
            s.push_str(
                "<sheetPr filterMode=\"false\"><pageSetUpPr fitToPage=\"false\"/></sheetPr>",
            );
        }
        // Only a buffered (auto-sized) sheet knows its size before its rows.
        if let Some(auto) = sheet.auto_size.as_ref().filter(|a| a.max_cells > 0) {
            let mut last = String::new();
            column_letters(auto.max_cells - 1, &mut last);
            let _ = write!(s, "<dimension ref=\"A1:{last}{}\"/>", sheet.rows);
        }
        if let Some(v) = &sheet.view {
            s.push_str(&sheet_view_xml(v));
        }
        match &sheet.auto_size {
            None if !sheet.widths.is_empty() => {
                s.push_str("<cols>");
                for (min, max, width) in &sheet.widths {
                    let _ = write!(
                        s,
                        "<col min=\"{min}\" max=\"{max}\" width=\"{}\" customWidth=\"true\"/>",
                        number(*width)
                    );
                }
                s.push_str("</cols>");
            }
            None => {}
            Some(auto) => s.push_str(&auto_size_cols(&sheet.widths, &auto.widths)),
        }
        s.push_str("<sheetData>");
        self.zip().write_all(s.as_bytes())?;
        Ok(())
    }

    /// OpenSpout's `SheetManager::throwIfNameIsInvalid` (uniqueness is case-sensitive there).
    fn validate_name(&self, name: &str, index: usize) -> Result<()> {
        let mut failed = Vec::new();
        let len = name.chars().count();
        if self
            .sheets
            .iter()
            .enumerate()
            .any(|(i, s)| i != index && s.name == name)
        {
            failed.push("It should be unique");
        } else if len == 0 {
            failed.push("It should not be blank");
        } else {
            if len > 31 {
                failed.push("It should not exceed 31 characters");
            }
            if name.contains(['\\', '/', '?', '*', ':', '[', ']']) {
                failed.push("It should not contain these characters: \\ / ? * : [ or ]");
            }
            if name.starts_with('\'') || name.ends_with('\'') {
                failed.push("It should not start or end with a single quote");
            }
            if name.eq_ignore_ascii_case("History") {
                failed.push("It should not be a reserved name (\"History\")");
            }
        }
        if failed.is_empty() {
            Ok(())
        } else {
            Err(invalid(&format!(
                "The sheet's name (\"{name}\") is invalid. It did not respect these rules:\n - {}",
                failed.join("\n - ")
            )))
        }
    }

    fn content_types(&self) -> String {
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n    \
             <Default ContentType=\"application/xml\" Extension=\"xml\"/>\n    \
             <Default ContentType=\"application/vnd.openxmlformats-package.relationships+xml\" Extension=\"rels\"/>\n    \
             <Override ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\" PartName=\"/xl/workbook.xml\"/>",
        );
        for i in 1..=self.sheets.len() {
            let _ = write!(
                s,
                "<Override ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\" PartName=\"/xl/worksheets/sheet{i}.xml\"/>"
            );
        }
        s.push_str(
            "<Override ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\" PartName=\"/xl/styles.xml\"/>\n\
             <Override ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\" PartName=\"/docProps/core.xml\"/>\n\
             <Override ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\" PartName=\"/docProps/app.xml\"/></Types>",
        );
        s
    }

    fn workbook_xml(&self) -> String {
        let mut s = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <workbook xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\">    <sheets>"
        );
        for (i, sheet) in self.sheets.iter().enumerate() {
            let _ = write!(
                s,
                "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rIdSheet{}\" state=\"visible\"/>",
                escape(&sheet.name),
                i + 1,
                i + 1
            );
        }
        s.push_str("    </sheets>");
        // Excel needs the hidden _FilterDatabase name for each autofilter.
        let mut names = String::new();
        for (i, sheet) in self.sheets.iter().enumerate() {
            if let Some(f) = sheet.auto_filter {
                let _ = write!(
                    names,
                    "<definedName function=\"false\" hidden=\"true\" localSheetId=\"{i}\" \
                     name=\"_xlnm._FilterDatabase\" vbProcedure=\"false\">'{}'!{}</definedName>",
                    escape(&sheet.name),
                    f.absolute()
                );
            }
        }
        if !names.is_empty() {
            let _ = write!(s, "<definedNames>{names}</definedNames>");
        }
        s.push_str("</workbook>");
        s
    }

    fn workbook_rels(&self) -> String {
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n    \
             <Relationship Id=\"rIdStyles\" Target=\"styles.xml\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\"/>",
        );
        for i in 1..=self.sheets.len() {
            let _ = write!(
                s,
                "<Relationship Id=\"rIdSheet{i}\" Target=\"worksheets/sheet{i}.xml\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\"/>"
            );
        }
        s.push_str("</Relationships>");
        s
    }
}

impl Drop for XlsxWriter {
    /// A writer dropped without `close()` (e.g. after an exception) still produces a
    /// complete, valid file instead of a zip with no workbook. Errors are ignored.
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

impl Sheet {
    fn new(name: String) -> Self {
        Sheet {
            name,
            view: None,
            widths: Vec::new(),
            rows: 0,
            open: false,
            auto_filter: None,
            merges: Vec::new(),
            hyperlinks: String::new(),
            link_rels: String::new(),
            external_links: 0,
            auto_size: None,
        }
    }

    /// OpenSpout's `HyperlinkManager::addHyperlinks` for one cell.
    fn add_link(&mut self, cell_ref: &str, link: Link<'_>) {
        let tooltip = link
            .tooltip
            .map(|t| format!(" tooltip=\"{}\"", escape(t)))
            .unwrap_or_default();
        let location = link
            .target
            .strip_prefix('#')
            .or_else(|| link.target.strip_prefix("sheet://"));
        match location {
            Some(loc) => {
                let _ = write!(
                    self.hyperlinks,
                    "<hyperlink ref=\"{cell_ref}\" location=\"{}\"{tooltip}/>",
                    escape(loc)
                );
            }
            None => {
                self.external_links += 1;
                let n = self.external_links;
                let _ = write!(
                    self.hyperlinks,
                    "<hyperlink ref=\"{cell_ref}\" r:id=\"rId_hyperlink{n}\"{tooltip}/>"
                );
                let _ = writeln!(
                    self.link_rels,
                    "  <Relationship Id=\"rId_hyperlink{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
                    escape(link.target)
                );
            }
        }
    }
}

/// `<cols>` for an auto-sized sheet, ascending: explicit widths win, then measured
/// ones (marked bestFit), consecutive columns with the same width in one range.
fn auto_size_cols(explicit: &[(u32, u32, f64)], measured: &[Option<f64>]) -> String {
    let mut cols: std::collections::BTreeMap<u32, (f64, bool)> = std::collections::BTreeMap::new();
    for (i, w) in measured.iter().enumerate() {
        if let Some(w) = w {
            cols.insert(i as u32 + 1, (*w, true));
        }
    }
    for &(min, max, w) in explicit {
        for c in min..=max {
            cols.insert(c, (w, false));
        }
    }
    if cols.is_empty() {
        return String::new();
    }
    let mut s = String::from("<cols>");
    let mut iter = cols.into_iter().peekable();
    while let Some((start, (w, best))) = iter.next() {
        let mut end = start;
        while let Some(&(next, (nw, nb))) = iter.peek() {
            if next == end + 1 && nw == w && nb == best {
                end = next;
                iter.next();
            } else {
                break;
            }
        }
        let _ = write!(
            s,
            "<col min=\"{start}\" max=\"{end}\" width=\"{}\" customWidth=\"true\"{}/>",
            number(w),
            if best { " bestFit=\"true\"" } else { "" }
        );
    }
    s.push_str("</cols>");
    s
}

/// The width PhpSpreadsheet's auto-size gives one cell, or `None` if it doesn't
/// count: empty cells, formulas (PhpSpreadsheet would measure the calculated value,
/// which the writer doesn't know), and cells in a merge (except the first cell of a
/// one-column merge). Merges count when declared before the row is written.
fn measure(
    data: &CellData<'_>,
    style: &Style,
    col: u32,
    row: u32,
    filter: Option<&CellRange>,
    merges: &[CellRange],
) -> Option<f64> {
    let merged = merges.iter().find(|m| {
        (m.first_col..=m.last_col).contains(&col) && (m.first_row..=m.last_row).contains(&row)
    });
    if let Some(m) = merged
        && !(m.first_col == m.last_col && m.first_col == col && m.first_row == row)
    {
        return None;
    }
    let format = style.format.as_deref();
    let text = match *data {
        CellData::Empty | CellData::Formula(_) => return None,
        CellData::Str(s) => std::borrow::Cow::Borrowed(s),
        CellData::Int(i) if format.is_none_or(|f| f == "General") => i.to_string().into(),
        CellData::Int(i) => autosize::format_number(i as f64, format).into(),
        CellData::Float(f) => autosize::format_number(f, format).into(),
        CellData::Bool(b) => std::borrow::Cow::Borrowed(if b { "TRUE" } else { "FALSE" }),
    };
    if text.is_empty() {
        return None;
    }
    let fallback = Style::fallback();
    Some(autosize::column_width(
        &text,
        &Measure {
            font: autosize::Font {
                name: &style.font_name,
                size: style.font_size as f64,
            },
            default_font: autosize::Font {
                name: &fallback.font_name,
                size: fallback.font_size as f64,
            },
            rotation: style.text_rotation,
            filter: filter
                .is_some_and(|f| f.first_row == row && (f.first_col..=f.last_col).contains(&col)),
            indent: i64::from(style.horizontal.as_deref() == Some("center")),
        },
    ))
}

fn parse_range_arg(range: &str) -> Result<CellRange> {
    CellRange::parse(range).ok_or_else(|| {
        Error::InvalidArgument(format!(
            "'{range}' is not a cell range like A1:C10 (columns A–XFD, rows 1–{MAX_ROWS})"
        ))
    })
}

/// OpenSpout `SheetView::getXml()` with its default attributes.
fn sheet_view_xml(v: &SheetView) -> String {
    let mut s = format!(
        "<sheetViews><sheetView showFormulas=\"false\" showGridLines=\"{}\" showRowColHeaders=\"true\" \
         showZeros=\"true\" rightToLeft=\"false\" tabSelected=\"false\" showOutlineSymbols=\"true\" \
         defaultGridColor=\"true\" view=\"normal\" topLeftCell=\"A1\" colorId=\"64\" zoomScale=\"100\" \
         zoomScaleNormal=\"100\" zoomScalePageLayoutView=\"100\" workbookViewId=\"0\">",
        v.show_grid_lines
    );
    // OpenSpout's freezeRow N means "rows above N are frozen", i.e. N - 1 rows.
    if v.freeze_rows > 0 || v.freeze_columns > 0 {
        let mut letters = String::new();
        column_letters(v.freeze_columns as usize, &mut letters);
        s.push_str("<pane");
        if v.freeze_columns > 0 {
            let _ = write!(s, " xSplit=\"{}\"", v.freeze_columns);
        }
        if v.freeze_rows > 0 {
            let _ = write!(s, " ySplit=\"{}\"", v.freeze_rows);
        }
        let pane = match (v.freeze_columns > 0, v.freeze_rows > 0) {
            (true, true) => "bottomRight",
            (true, false) => "topRight",
            _ => "bottomLeft",
        };
        let _ = write!(
            s,
            " topLeftCell=\"{letters}{}\" activePane=\"{pane}\" state=\"frozen\"/>",
            v.freeze_rows + 1
        );
    }
    s.push_str("</sheetView></sheetViews>");
    s
}

/// 0-based column index → letters (0 → A).
fn column_letters(mut idx: usize, out: &mut String) {
    let start = out.len();
    loop {
        out.push((b'A' + (idx % 26) as u8) as char);
        if idx < 26 {
            break;
        }
        idx = idx / 26 - 1;
    }
    // Letters were pushed least-significant first; they're ASCII, so reverse in place.
    let tail: String = out[start..].chars().rev().collect();
    out.truncate(start);
    out.push_str(&tail);
}

/// A number as written in XML: shortest round-trip form, integral values without
/// a fractional part (PHP prints `3.0` as `3`), PHP spellings for non-finite values.
pub fn number(f: f64) -> String {
    if f.is_nan() {
        "NAN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "INF".into() } else { "-INF".into() }
    } else {
        format!("{f}")
    }
}

fn invalid(message: &str) -> Error {
    Error::InvalidData {
        part: "writer".into(),
        message: message.to_string(),
    }
}

const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdWorkbook\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\n\
<Relationship Id=\"rIdCore\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\n\
<Relationship Id=\"rIdApp\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>";

const APP_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\">\n    \
<Application>rusty_sheet</Application>\n    <TotalTime>0</TotalTime>\n</Properties>";

fn core_xml() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let p = crate::xlsx::dates::serial_to_parts(25_569.0 + now as f64 / 86_400.0);
    let stamp = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
        p.year, p.month, p.day, p.hour, p.minute, p.second
    );
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n    \
<dc:title>Untitled Spreadsheet</dc:title>\n    <dc:subject></dc:subject>\n    <dc:creator>rusty_sheet</dc:creator>\n    \
<cp:lastModifiedBy>rusty_sheet</cp:lastModifiedBy>\n    <cp:keywords></cp:keywords>\n    <dc:description></dc:description>\n    \
<cp:category></cp:category>\n    <dc:language></dc:language>\n    \
<dcterms:created xsi:type=\"dcterms:W3CDTF\">{stamp}</dcterms:created>\n    \
<dcterms:modified xsi:type=\"dcterms:W3CDTF\">{stamp}</dcterms:modified>\n    <cp:revision>0</cp:revision>\n</cp:coreProperties>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_like_php_where_it_matters() {
        assert_eq!(number(3.0), "3");
        assert_eq!(number(0.1), "0.1");
        assert_eq!(number(1.0 / 3.0), "0.3333333333333333");
        assert_eq!(number(-0.0), "-0");
        assert_eq!(number(f64::NAN), "NAN");
        assert_eq!(number(f64::NEG_INFINITY), "-INF");
    }

    #[test]
    fn letters() {
        let l = |i| {
            let mut s = String::new();
            column_letters(i, &mut s);
            s
        };
        assert_eq!(l(0), "A");
        assert_eq!(l(25), "Z");
        assert_eq!(l(26), "AA");
        assert_eq!(l(16_383), "XFD");
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rs_writer_{}_{name}.xlsx", std::process::id()))
    }

    fn cell(s: &str) -> Cell<'_> {
        Cell {
            data: CellData::Str(s),
            style: None,
            multiline: false,
            link: None,
        }
    }

    fn read_names(path: &std::path::Path) -> Vec<String> {
        crate::xlsx::XlsxReader::open(path)
            .unwrap()
            .sheet_names()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn default_sheet_name_must_be_unique() {
        let path = tmp("dupe");
        let mut w = XlsxWriter::create(&path).unwrap();
        w.add_sheet(Some("Sheet2")).unwrap();
        let err = w.add_sheet(None).unwrap_err().to_string();
        assert!(err.contains("It should be unique"), "{err}");
        drop(w);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn limits_are_errors_not_panics() {
        let path = tmp("limits");
        let mut w = XlsxWriter::create(&path).unwrap();
        assert!(w.freeze(MAX_ROWS, 0).is_err());
        assert!(w.freeze(0, MAX_COLUMNS as u32).is_err());
        assert!(w.freeze(u32::MAX, u32::MAX).is_err());
        assert!(w.set_column_width(10.0, &[0]).is_err());
        assert!(w.set_column_width(10.0, &[MAX_COLUMNS as u32 + 1]).is_err());
        let wide: Vec<Cell<'_>> = (0..=MAX_COLUMNS).map(|_| cell("x")).collect();
        assert!(w.add_row(&wide, None).is_err());
        // The largest legal row still works.
        assert!(w.add_row(&wide[..MAX_COLUMNS], None).is_ok());
        drop(w);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn failed_row_leaves_no_trace() {
        let path = tmp("atomic");
        let mut w = XlsxWriter::create(&path).unwrap();
        let bad = [Cell {
            data: CellData::Str("a"),
            style: Some(99),
            multiline: false,
            link: None,
        }];
        assert!(w.add_row(&bad, None).is_err());
        // No row number used and the sheet isn't started: setup is still allowed.
        assert!(w.set_column_width(10.0, &[1]).is_ok());
        w.add_row(&[cell("b")], None).unwrap();
        w.close().unwrap();
        let mut r = crate::xlsx::XlsxReader::open(&path).unwrap();
        let mut rows = r.rows(0, Default::default()).unwrap();
        let mut row = crate::xlsx::Row::default();
        assert!(rows.next_row(&mut row).unwrap());
        assert_eq!(row.number, 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dropping_without_close_still_finishes_the_file() {
        let path = tmp("drop");
        {
            let mut w = XlsxWriter::create(&path).unwrap();
            w.add_sheet(Some("Kept")).unwrap();
            w.add_row(&[cell("x")], None).unwrap();
        }
        assert_eq!(read_names(&path), ["Kept"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn continues_on_a_new_sheet_at_the_row_limit() {
        let path = tmp("rowlimit");
        let mut w = XlsxWriter::create(&path).unwrap();
        for _ in 0..MAX_ROWS {
            w.add_row(&[], None).unwrap(); // empty rows: cheap, still numbered
        }
        w.add_row(&[cell("overflow")], None).unwrap();
        w.close().unwrap();
        assert_eq!(read_names(&path), ["Sheet1", "Sheet2"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn column_letters_never_panic() {
        let mut s = String::new();
        column_letters(475_254, &mut s);
        assert_eq!(s, "AAAAA");
    }

    #[test]
    fn freeze_pane_matches_openspout() {
        // OpenSpout withFreezeRow(2): ySplit=1, topLeftCell=A2.
        let v = SheetView {
            show_grid_lines: false,
            freeze_rows: 1,
            freeze_columns: 0,
        };
        assert!(sheet_view_xml(&v).contains(
            "<pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/>"
        ));
        assert!(sheet_view_xml(&v).contains("showGridLines=\"false\""));
    }

    #[test]
    fn cell_ranges() {
        let r = |s| CellRange::parse(s).map(|r| r.to_string());
        assert_eq!(r("A1:C3").as_deref(), Some("A1:C3"));
        assert_eq!(r("$c$3:$a$1").as_deref(), Some("A1:C3"));
        assert_eq!(r(" B2 ").as_deref(), Some("B2:B2"));
        assert_eq!(r("XFD1048576:A1").as_deref(), Some("A1:XFD1048576"));
        for bad in [
            "", "A", "1", "A0", "XFE1", "A1048577", "AAAA1", "A1:", "A1:B2:C3", "A-1", "$$A1",
            "A$$1",
        ] {
            assert_eq!(r(bad), None, "{bad}");
        }
        let a = CellRange::parse("B2:C3").unwrap();
        assert!(a.overlaps(&CellRange::parse("C3:D4").unwrap()));
        assert!(!a.overlaps(&CellRange::parse("D1:D9").unwrap()));
        assert_eq!(a.absolute(), "$B$2:$C$3");
    }

    #[test]
    fn merges_are_validated_and_written_after_the_rows() {
        let path = tmp("merges");
        let mut w = XlsxWriter::create(&path).unwrap();
        let invalid = |e: Error| matches!(e, Error::InvalidArgument(_));
        assert!(invalid(w.merge_cells("A1").unwrap_err()));
        assert!(invalid(w.merge_cells("nope").unwrap_err()));
        w.merge_cells("A1:C1").unwrap();
        assert!(invalid(w.merge_cells("B1:B2").unwrap_err()));
        w.add_row(&[cell("title")], None).unwrap();
        w.merge_cells("A3:A4").unwrap(); // after rows is fine
        w.close().unwrap();
        let sheet = part(&path, "xl/worksheets/sheet1.xml");
        assert!(sheet.ends_with(
            "</sheetData><mergeCells count=\"2\"><mergeCell ref=\"A1:C1\"/><mergeCell ref=\"A3:A4\"/></mergeCells></worksheet>"
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn autofilter_needs_setting_before_rows() {
        let path = tmp("filter");
        let mut w = XlsxWriter::create(&path).unwrap();
        w.add_sheet(Some("It's")).unwrap();
        w.set_auto_filter("A1:B10").unwrap();
        w.add_row(&[cell("x")], None).unwrap();
        assert!(w.set_auto_filter("A1:B2").is_err());
        w.close().unwrap();
        let sheet = part(&path, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains(
            "<sheetPr filterMode=\"false\"><pageSetUpPr fitToPage=\"false\"/></sheetPr><sheetData>"
        ));
        assert!(sheet.contains("</sheetData><autoFilter ref=\"A1:B10\"/></worksheet>"));
        assert!(part(&path, "xl/workbook.xml").contains(
            "<definedNames><definedName function=\"false\" hidden=\"true\" localSheetId=\"0\" name=\"_xlnm._FilterDatabase\" vbProcedure=\"false\">'It's'!$A$1:$B$10</definedName></definedNames></workbook>"
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn hyperlinks_to_urls_and_places() {
        let path = tmp("links");
        let mut w = XlsxWriter::create(&path).unwrap();
        let linked = |text, target, tooltip| Cell {
            link: Some(Link { target, tooltip }),
            ..cell(text)
        };
        w.add_row(
            &[
                linked("site", "https://example.com/?a=1&b=2", Some("Go <there>")),
                linked("sheet", "#'Other sheet'!A1", None),
                linked("php", "sheet://'Other sheet'!B2", None),
                linked("again", "mailto:x@example.com", None),
            ],
            None,
        )
        .unwrap();
        let mut not_text = linked("x", "#A1", None);
        not_text.data = CellData::Int(1);
        assert!(matches!(
            w.add_row(&[not_text], None),
            Err(Error::InvalidArgument(_))
        ));
        w.add_sheet(Some("Other sheet")).unwrap(); // the first sheet ends here
        w.close().unwrap();
        let sheet = part(&path, "xl/worksheets/sheet1.xml");
        assert!(sheet.ends_with(concat!(
            "</sheetData><hyperlinks>",
            "<hyperlink ref=\"A1\" r:id=\"rId_hyperlink1\" tooltip=\"Go &lt;there&gt;\"/>",
            "<hyperlink ref=\"B1\" location=\"'Other sheet'!A1\"/>",
            "<hyperlink ref=\"C1\" location=\"'Other sheet'!B2\"/>",
            "<hyperlink ref=\"D1\" r:id=\"rId_hyperlink2\"/>",
            "</hyperlinks></worksheet>"
        )));
        let rels = part(&path, "xl/worksheets/_rels/sheet1.xml.rels");
        assert!(rels.contains("Id=\"rId_hyperlink1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"https://example.com/?a=1&amp;b=2\" TargetMode=\"External\"/>\n"));
        assert!(rels.ends_with("TargetMode=\"External\"/>\n</Relationships>"));
        // A sheet without links to URLs has no .rels part.
        assert!(!zip_has(&path, "xl/worksheets/_rels/sheet2.xml.rels"));
        let _ = std::fs::remove_file(path);
    }

    fn part(path: &std::path::Path, name: &str) -> String {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut zip.by_name(name).unwrap(), &mut s).unwrap();
        s
    }

    fn zip_has(path: &std::path::Path, name: &str) -> bool {
        let zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        zip.index_for_name(name).is_some()
    }

    fn temp_files(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).unwrap().count()
    }

    #[test]
    fn auto_size_buffers_rows_and_writes_widths_first() {
        let dir = std::env::temp_dir().join(format!("rs_autosize_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |auto: bool, path: &std::path::Path| {
            let mut w = XlsxWriter::create(path).unwrap();
            w.set_temp_dir(&dir);
            if auto {
                w.auto_size_columns(None).unwrap();
                w.set_column_width(30.0, &[3]).unwrap();
            }
            w.set_auto_filter("A1:B1").unwrap();
            w.merge_cells("A3:B3").unwrap();
            w.add_row(&[cell("Name"), cell("Description"), cell("x")], None)
                .unwrap();
            w.add_row(&[cell("Alexandra"), cell("short"), cell("y")], None)
                .unwrap();
            w.add_row(&[cell("a merged title that is long"), cell("")], None)
                .unwrap();
            if auto {
                assert_eq!(temp_files(&dir), 1, "rows go to a temp file");
            }
            w.close().unwrap();
        };
        let (streamed, buffered) = (tmp("as_stream"), tmp("as_buffer"));
        write(false, &streamed);
        write(true, &buffered);
        assert_eq!(temp_files(&dir), 0, "the temp file is removed");

        let a = part(&streamed, "xl/worksheets/sheet1.xml");
        let b = part(&buffered, "xl/worksheets/sheet1.xml");
        let rows = |s: &str| s[s.find("<sheetData>").unwrap()..].to_string();
        assert_eq!(rows(&a), rows(&b), "same rows either way");
        // Widths computed by PhpSpreadsheet 5.11 (approx mode, Calibri 12 default font):
        // the header cells get filter-dropdown room, and the merged cell doesn't count.
        assert!(b.contains("<dimension ref=\"A1:C3\"/>"), "{b}");
        assert!(b.contains(concat!(
            "<cols><col min=\"1\" max=\"1\" width=\"11.521\" customWidth=\"true\" bestFit=\"true\"/>",
            "<col min=\"2\" max=\"2\" width=\"16.234\" customWidth=\"true\" bestFit=\"true\"/>",
            "<col min=\"3\" max=\"3\" width=\"30\" customWidth=\"true\"/></cols><sheetData>"
        )), "{b}");
        assert!(!a.contains("<dimension") && !a.contains("<cols>"));

        // Dropped without close(): the file is still finished and the temp file removed.
        let dropped = tmp("as_drop");
        {
            let mut w = XlsxWriter::create(&dropped).unwrap();
            w.set_temp_dir(&dir);
            w.auto_size_columns(Some(&[2])).unwrap();
            w.add_row(&[cell("not sized"), cell("sized")], None)
                .unwrap();
        }
        assert_eq!(temp_files(&dir), 0);
        let d = part(&dropped, "xl/worksheets/sheet1.xml");
        assert!(d.contains("<cols><col min=\"2\" max=\"2\""), "{d}");
        assert!(w_errors_are_arguments());
        for p in [streamed, buffered, dropped] {
            let _ = std::fs::remove_file(p);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn w_errors_are_arguments() -> bool {
        let path = tmp("as_args");
        let mut w = XlsxWriter::create(&path).unwrap();
        let bad = matches!(
            w.auto_size_columns(Some(&[0])),
            Err(Error::InvalidArgument(_))
        );
        w.add_row(&[cell("x")], None).unwrap();
        let late = matches!(w.auto_size_columns(None), Err(Error::InvalidData { .. }));
        drop(w);
        let _ = std::fs::remove_file(path);
        bad && late
    }

    #[test]
    fn unwritable_temp_dir_is_an_io_error() {
        let path = tmp("as_tempdir");
        let mut w = XlsxWriter::create(&path).unwrap();
        w.set_temp_dir(std::path::Path::new("/nonexistent/rusty_sheet"));
        w.auto_size_columns(None).unwrap();
        assert!(matches!(w.add_row(&[cell("x")], None), Err(Error::Io(_))));
        drop(w);
        let _ = std::fs::remove_file(path);
    }
}
