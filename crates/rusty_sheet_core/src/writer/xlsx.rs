//! Streaming XLSX writer producing the same XML as OpenSpout's XLSX writer.
//!
//! Rows go straight into the zip, one sheet after another, so memory stays flat
//! and nothing is buffered on disk. The differences from OpenSpout are deliberate:
//! - no `<dimension>` (it would need the whole sheet buffered first; it's optional);
//! - no empty comments / VML / shared-strings parts (OpenSpout always writes them);
//! - numbers are written with full precision; OpenSpout goes through PHP's
//!   `precision` ini setting and keeps 14 significant digits (`1/3` → `0.33333333333333`).

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use zip::ZipWriter;
use zip::write::SimpleFileOptions;

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
        })
    }

    /// Register a style; the returned handle is used in [`Cell::style`].
    /// Like OpenSpout (object identity), every call is a distinct style.
    pub fn add_style(&mut self, style: Style) -> usize {
        self.styles.push(style);
        self.styles.len() - 1
    }

    /// The style registered under `handle`.
    pub fn style(&self, handle: usize) -> Option<&Style> {
        self.styles.get(handle)
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
            col_letters.clear();
            column_letters(col, &mut col_letters);
            let _ = write!(
                self.buf,
                "<c r=\"{col_letters}{row_number}\" s=\"{style_id}\""
            );
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
                self.zip().write_all(b"</sheetData></worksheet>")?;
                self.sheets[i].open = true;
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

    /// Whether `close()` (or the final write) has happened.
    pub fn is_closed(&self) -> bool {
        self.closed
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
            self.start_sheet_part(i)?;
            self.sheets[i].open = true;
        }
        Ok(())
    }

    fn finish_current_sheet(&mut self) -> Result<()> {
        if self.sheets.last().is_some_and(|s| s.open) {
            self.zip().write_all(b"</sheetData></worksheet>")?;
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
        if let Some(v) = &sheet.view {
            s.push_str(&sheet_view_xml(v));
        }
        if !sheet.widths.is_empty() {
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
        s.push_str("    </sheets></workbook>");
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
        }
    }
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
}
