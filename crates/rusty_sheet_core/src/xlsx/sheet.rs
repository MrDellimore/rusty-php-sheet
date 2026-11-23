//! Streaming `<sheetData>` parser: one row at a time, constant memory.
//!
//! Cell semantics follow OpenSpout's `CellValueFormatter::extractAndFormatNodeValue`
//! (see `docs/parity-notes.md`), with three deliberate deviations:
//! - unparseable numbers become `Null` instead of `0.0`;
//! - shared-formula children get their real formula instead of `"="`;
//! - rows aren't padded to the `<dimension>` width, and trailing blanks are trimmed
//!   (sheet-stream trims them anyway, so its output is unchanged).

use std::collections::HashMap;
use std::io::BufReader;
use std::sync::Arc;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::dates::{SYSTEM_1904_OFFSET, is_valid_serial};
use super::formula;
use super::package::PartReader;
use super::shared_strings::SharedStrings;
use super::styles::{CellFormat, Styles, php_bool, php_int};
use super::unescape::unescape;
use super::xml;
use crate::value::{CellValue, DateKind};
use crate::{Error, Result};

/// Excel's column limit (XFD). Larger references are treated as corrupt.
const MAX_COLUMNS: usize = 16_384;
/// Excel's last row. Higher row numbers are a row error, like columns past XFD.
const MAX_ROWS: u32 = 1_048_576;

#[derive(Debug, Clone, Copy, Default)]
pub struct ReadOptions {
    /// Return rows with no values (as empty rows) instead of skipping them, and fill
    /// gaps in the row numbering with empty rows. OpenSpout's `SHOULD_PRESERVE_EMPTY_ROWS`.
    pub preserve_empty_rows: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    /// 1-based row number from the sheet (`<row r="…">`).
    pub number: u32,
    /// Cells from column A, trimmed after the last non-blank cell.
    pub cells: Vec<CellValue>,
}

/// Where character data currently goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Capture {
    None,
    V,
    F,
    InlineT,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellType {
    Number,
    SharedString,
    InlineString,
    Str,
    Bool,
    Error,
    IsoDate,
    Unknown,
}

impl CellType {
    fn parse(t: Option<&str>) -> Self {
        match t {
            None | Some("") | Some("n") => CellType::Number,
            Some("s") => CellType::SharedString,
            Some("inlineStr") => CellType::InlineString,
            Some("str") => CellType::Str,
            Some("b") => CellType::Bool,
            Some("e") => CellType::Error,
            Some("d") => CellType::IsoDate,
            Some(_) => CellType::Unknown,
        }
    }
}

struct SharedFormula {
    formula: String,
    row: u32,
    col: usize,
}

/// The cell currently being parsed.
struct CellState {
    col: usize,
    kind: CellType,
    style: u32,
    has_v: bool,
    has_f: bool,
    f_shared: bool,
    f_si: Option<u32>,
    f_has_ref: bool,
    v: String,
    f: String,
    inline: String,
    /// Where the current inline `<t>` starts in `inline` (each `<t>` is unescaped on its own).
    inline_t_start: usize,
}

impl CellState {
    fn new() -> Self {
        CellState {
            col: 0,
            kind: CellType::Number,
            style: 0,
            has_v: false,
            has_f: false,
            f_shared: false,
            f_si: None,
            f_has_ref: false,
            v: String::with_capacity(64),
            f: String::new(),
            inline: String::new(),
            inline_t_start: 0,
        }
    }

    fn reset(&mut self, col: usize, kind: CellType, style: u32) {
        self.col = col;
        self.kind = kind;
        self.style = style;
        self.has_v = false;
        self.has_f = false;
        self.f_shared = false;
        self.f_si = None;
        self.f_has_ref = false;
        self.v.clear();
        self.f.clear();
        self.inline.clear();
    }
}

pub struct SheetRows {
    reader: Option<Reader<BufReader<PartReader>>>,
    part: String,
    ev: Vec<u8>,
    shared: Arc<SharedStrings>,
    styles: Arc<Styles>,
    date1904: bool,
    opts: ReadOptions,

    capture: Capture,
    in_cell: bool,
    cell: CellState,
    shared_formulas: HashMap<u32, SharedFormula>,

    /// Row number of the last `<row>` parsed (for rows without `r`).
    last_row: u32,
    /// Preserve mode: the next row number to hand out.
    next_number: u32,
    /// Preserve mode: a parsed row held back while gap rows are emitted.
    held: Option<Row>,
}

impl SheetRows {
    pub(crate) fn new(
        stream: Option<PartReader>,
        part: String,
        shared: Arc<SharedStrings>,
        styles: Arc<Styles>,
        date1904: bool,
        opts: ReadOptions,
    ) -> Self {
        SheetRows {
            reader: stream.map(|s| xml::reader(BufReader::with_capacity(xml::XML_BUF, s))),
            part,
            ev: Vec::with_capacity(8 * 1024),
            shared,
            styles,
            date1904,
            opts,
            capture: Capture::None,
            in_cell: false,
            cell: CellState::new(),
            shared_formulas: HashMap::new(),
            last_row: 0,
            next_number: 1,
            held: None,
        }
    }

    /// Resolve `CellValue::SharedString` indexes.
    pub fn shared_strings(&self) -> &SharedStrings {
        &self.shared
    }

    /// The shared-string table, as an `Arc` so callers can hold it while iterating.
    pub fn shared_strings_arc(&self) -> &Arc<SharedStrings> {
        &self.shared
    }

    /// The styles, as an `Arc` so callers can hold them while iterating.
    pub fn styles_arc(&self) -> &Arc<Styles> {
        &self.styles
    }

    /// Read the next row into `out`, reusing its allocation. Returns `false` at the end.
    pub fn next_row(&mut self, out: &mut Row) -> Result<bool> {
        if !self.opts.preserve_empty_rows {
            loop {
                if !self.parse_row(out)? {
                    return Ok(false);
                }
                if !out.cells.is_empty() {
                    return Ok(true);
                }
            }
        }

        // Preserve mode: emit gap rows first, then the held row.
        if let Some(held) = &self.held {
            if self.next_number < held.number {
                out.number = self.next_number;
                out.cells.clear();
                self.next_number += 1;
                return Ok(true);
            }
            let held = self.held.take().expect("checked");
            self.next_number = held.number + 1;
            *out = held;
            return Ok(true);
        }
        if !self.parse_row(out)? {
            return Ok(false);
        }
        // OpenSpout's counter only moves forward: a row numbered at or below one
        // already emitted becomes an empty row at the next index (its data dropped).
        // Moving back instead would re-emit every gap after it.
        if out.number < self.next_number {
            out.cells.clear();
            out.number = self.next_number;
        } else if out.number > self.next_number {
            self.held = Some(std::mem::take(out));
            out.number = self.next_number;
        }
        self.next_number += 1;
        Ok(true)
    }

    /// Parse the next `<row>` element. Returns `false` at the end of the sheet.
    ///
    /// The reader and event buffer are moved out of `self` for the duration, so the
    /// loop can call `&mut self` helpers while an event borrows the buffer.
    fn parse_row(&mut self, out: &mut Row) -> Result<bool> {
        let Some(mut reader) = self.reader.take() else {
            return Ok(false);
        };
        let mut ev = std::mem::take(&mut self.ev);
        let result = self.parse_row_with(&mut reader, &mut ev, out);
        self.ev = ev;
        // Keep going after a row-level data error (the rest of the sheet is still
        // readable); stop for good at the end or after I/O and XML errors.
        if matches!(result, Ok(true) | Err(Error::InvalidData { .. })) {
            self.reader = Some(reader);
        }
        result
    }

    fn parse_row_with(
        &mut self,
        reader: &mut Reader<BufReader<PartReader>>,
        ev: &mut Vec<u8>,
        out: &mut Row,
    ) -> Result<bool> {
        out.cells.clear();
        self.in_cell = false;
        self.capture = Capture::None;
        let mut in_row = false;

        loop {
            ev.clear();
            let event = reader.read_event_into(ev).map_err(|source| Error::Xml {
                part: self.part.clone(),
                source,
            })?;
            match event {
                Event::Start(e) => match xml::local_name(&e) {
                    "c" if in_row => {
                        let (col, kind, style) = cell_attrs(&e, out.cells.len(), &self.part)?;
                        self.cell.reset(col, kind, style);
                        self.in_cell = true;
                    }
                    "v" if self.in_cell => {
                        self.cell.has_v = true;
                        self.capture = Capture::V;
                    }
                    "f" if self.in_cell => {
                        read_formula_attrs(&e, &mut self.cell);
                        self.capture = Capture::F;
                    }
                    // Like OpenSpout, every <t> under <is> counts, phonetic runs included.
                    "t" if self.in_cell && self.cell.kind == CellType::InlineString => {
                        self.cell.inline_t_start = self.cell.inline.len();
                        self.capture = Capture::InlineT;
                    }
                    "row" => {
                        out.number = row_number(&e, self.last_row, &self.part)?;
                        self.last_row = out.number;
                        in_row = true;
                    }
                    _ => {}
                },
                Event::Empty(e) => match xml::local_name(&e) {
                    "c" if in_row => {
                        // A self-closing cell has no value.
                        let (col, _, _) = cell_attrs(&e, out.cells.len(), &self.part)?;
                        place(&mut out.cells, col, CellValue::Empty);
                    }
                    "v" if self.in_cell => self.cell.has_v = true,
                    "f" if self.in_cell => read_formula_attrs(&e, &mut self.cell),
                    "row" => {
                        out.number = row_number(&e, self.last_row, &self.part)?;
                        self.last_row = out.number;
                        return Ok(true);
                    }
                    _ => {}
                },
                Event::Text(t) => match self.capture {
                    Capture::V => xml::push_text(&mut self.cell.v, &t),
                    Capture::F => xml::push_text(&mut self.cell.f, &t),
                    Capture::InlineT => xml::push_text(&mut self.cell.inline, &t),
                    Capture::None => {}
                },
                Event::GeneralRef(r) => match self.capture {
                    Capture::V => xml::push_ref(&mut self.cell.v, &r),
                    Capture::F => xml::push_ref(&mut self.cell.f, &r),
                    Capture::InlineT => xml::push_ref(&mut self.cell.inline, &r),
                    Capture::None => {}
                },
                Event::CData(c) => match self.capture {
                    Capture::V => self.cell.v.push_str(&c),
                    Capture::F => self.cell.f.push_str(&c),
                    Capture::InlineT => self.cell.inline.push_str(&c),
                    Capture::None => {}
                },
                Event::End(e) => match xml::end_local_name(e.name().0) {
                    "t" if self.capture == Capture::InlineT => {
                        let c = &mut self.cell;
                        let segment = c.inline.split_off(c.inline_t_start);
                        c.inline.push_str(&unescape(&segment));
                        self.capture = Capture::None;
                    }
                    "v" | "f" | "t" => self.capture = Capture::None,
                    "c" if self.in_cell => {
                        self.in_cell = false;
                        self.capture = Capture::None;
                        let value = self.finish_cell(out.number);
                        place(&mut out.cells, self.cell.col, value);
                    }
                    "row" if in_row => {
                        trim_trailing_blanks(&mut out.cells, &self.shared);
                        return Ok(true);
                    }
                    "sheetData" => return Ok(false),
                    _ => {}
                },
                Event::Eof => return Ok(false),
                _ => {}
            }
        }
    }

    /// Build the value of the cell that just ended (OpenSpout's `extractAndFormatNodeValue`).
    fn finish_cell(&mut self, row: u32) -> CellValue {
        let c = &self.cell;
        if c.has_f {
            let has_cached = c.kind == CellType::InlineString
                || !c.v.is_empty()
                || (c.has_v && c.kind == CellType::Str);
            let cached = if has_cached {
                match self.raw_value() {
                    CellValue::Null => None,
                    v => Some(Box::new(v)),
                }
            } else {
                None
            };
            let formula = self.formula_text(row);
            return CellValue::Formula { formula, cached };
        }
        if c.v.is_empty() && c.kind != CellType::InlineString {
            return CellValue::Empty;
        }
        self.raw_value()
    }

    /// The formula text, expanding shared-formula children from their master.
    fn formula_text(&mut self, row: u32) -> String {
        let c = &mut self.cell;
        if !c.f_shared {
            return std::mem::take(&mut c.f);
        }
        let Some(si) = c.f_si else {
            return std::mem::take(&mut c.f);
        };
        if c.f_has_ref || !c.f.is_empty() {
            // The master cell: remember it for the children.
            self.shared_formulas.insert(
                si,
                SharedFormula {
                    formula: c.f.clone(),
                    row,
                    col: c.col,
                },
            );
            return std::mem::take(&mut c.f);
        }
        match self.shared_formulas.get(&si) {
            Some(master) => formula::shift(
                &master.formula,
                i64::from(row) - i64::from(master.row),
                c.col as i64 - master.col as i64,
            ),
            None => String::new(), // master missing: same as OpenSpout ("=")
        }
    }

    /// OpenSpout's `formatRawValueForCellType`.
    fn raw_value(&mut self) -> CellValue {
        let c = &mut self.cell;
        match c.kind {
            CellType::InlineString => CellValue::String(std::mem::take(&mut c.inline)),
            CellType::SharedString => {
                let idx = php_int(&c.v);
                match u32::try_from(idx) {
                    Ok(i) if (i as usize) < self.shared.len() => CellValue::SharedString(i),
                    _ => CellValue::Null,
                }
            }
            // OpenSpout: trim() then unescape.
            CellType::Str => CellValue::String(unescape(xml::php_trim(&c.v)).into_owned()),
            CellType::Bool => CellValue::Bool(php_bool(&c.v)),
            CellType::IsoDate => CellValue::IsoDate(std::mem::take(&mut c.v)),
            CellType::Error | CellType::Unknown => CellValue::Null,
            CellType::Number => {
                let Some(n) = parse_number(&c.v) else {
                    return CellValue::Null;
                };
                match self.styles.cell_format(c.style) {
                    CellFormat::General => match n {
                        Number::Int(i) => CellValue::Int(i),
                        Number::Float(f) => CellValue::number(f),
                    },
                    CellFormat::Date(DateKind::Duration, fmt) => CellValue::Date {
                        serial: n.as_f64(),
                        kind: DateKind::Duration,
                        fmt,
                    },
                    CellFormat::Date(DateKind::DateTime, fmt) => {
                        let serial = n.as_f64();
                        if !is_valid_serial(serial, self.date1904) {
                            return CellValue::Null;
                        }
                        let offset = if self.date1904 {
                            SYSTEM_1904_OFFSET
                        } else {
                            0.0
                        };
                        CellValue::Date {
                            serial: serial + offset,
                            kind: DateKind::DateTime,
                            fmt,
                        }
                    }
                }
            }
        }
    }
}

/// `r`, `t` and `s` of a `<c>` element. The column comes from `r` (letters only, as
/// OpenSpout does), or follows the previous cell when `r` is missing or unusable.
fn cell_attrs(e: &BytesStart<'_>, next_col: usize, part: &str) -> Result<(usize, CellType, u32)> {
    let mut col = None;
    let mut kind = None;
    let mut style = 0u32;
    let mut too_far = None;
    // Duplicate-attribute checks cost ~5% of total read time and buy us nothing.
    for a in e.attributes().with_checks(false).flatten() {
        match a.key.local_name().as_ref() {
            "r" => {
                col = column_from_ref(&a.value);
                if col.is_some_and(|c| c >= MAX_COLUMNS) {
                    too_far = Some(format!("cell reference `{}`", a.value));
                }
            }
            "t" => kind = Some(CellType::parse(Some(&a.value))),
            "s" => style = u32::try_from(php_int(&a.value)).unwrap_or(0),
            _ => {}
        }
    }
    let col = col.unwrap_or(next_col);
    if col >= MAX_COLUMNS {
        // Without `r`, the column follows the previous cell: column XFD + 1.
        let what = too_far.unwrap_or_else(|| format!("cell {}", MAX_COLUMNS + 1));
        return Err(Error::InvalidData {
            part: part.to_string(),
            message: format!("{what} is beyond column XFD"),
        });
    }
    Ok((col, kind.unwrap_or(CellType::Number), style))
}

fn read_formula_attrs(e: &BytesStart<'_>, cell: &mut CellState) {
    cell.has_f = true;
    // Duplicate-attribute checks cost ~5% of total read time and buy us nothing.
    for a in e.attributes().with_checks(false).flatten() {
        match a.key.local_name().as_ref() {
            "t" => cell.f_shared = &*a.value == "shared",
            "si" => cell.f_si = a.value.parse().ok(),
            "ref" => cell.f_has_ref = true,
            _ => {}
        }
    }
}

/// The row's `r` (PHP `(int)`), or the row after the previous one. A row past
/// Excel's last row is an error; the parser resumes at the next `<row>`.
fn row_number(e: &BytesStart<'_>, last: u32, part: &str) -> Result<u32> {
    let n = xml::attr(e, "r")
        .map(|v| php_int(&v))
        .filter(|&n| n > 0)
        .unwrap_or(i64::from(last) + 1);
    match u32::try_from(n) {
        Ok(n) if n <= MAX_ROWS => Ok(n),
        _ => Err(Error::InvalidData {
            part: part.to_string(),
            message: format!("row {n} is beyond row {MAX_ROWS}"),
        }),
    }
}

/// 0-based column index from the letters of a cell reference ("AB12" → 27).
fn column_from_ref(r: &str) -> Option<usize> {
    let letters = r.bytes().take_while(u8::is_ascii_alphabetic);
    let mut col = 0usize;
    let mut n = 0;
    for b in letters {
        col = col * 26 + usize::from(b.to_ascii_uppercase() - b'A' + 1);
        n += 1;
        if n > 3 {
            return Some(usize::MAX); // rejected by the MAX_COLUMNS check
        }
    }
    (n > 0).then(|| col - 1)
}

/// Put a value at `col`, filling any gap with `Empty`. A repeated column overwrites.
#[inline]
fn place(cells: &mut Vec<CellValue>, col: usize, value: CellValue) {
    if col == cells.len() {
        cells.push(value);
    } else if col > cells.len() {
        cells.resize(col, CellValue::Empty);
        cells.push(value);
    } else {
        cells[col] = value;
    }
}

/// Drop trailing cells that PHP would see as `''`.
fn trim_trailing_blanks(cells: &mut Vec<CellValue>, shared: &SharedStrings) {
    while let Some(last) = cells.last() {
        let blank = match last {
            CellValue::Empty => true,
            CellValue::String(s) | CellValue::IsoDate(s) => s.is_empty(),
            CellValue::SharedString(i) => shared.get(*i).is_some_and(str::is_empty),
            _ => false,
        };
        if !blank {
            break;
        }
        cells.pop();
    }
}

#[derive(Clone, Copy)]
enum Number {
    Int(i64),
    Float(f64),
}

impl Number {
    fn as_f64(self) -> f64 {
        match self {
            Number::Int(i) => i as f64,
            Number::Float(f) => f,
        }
    }
}

/// Parse a `<v>` number. Plain integers take a fast path; anything that isn't a
/// finite decimal number is `None` (→ `Null`, a deliberate deviation from PHP's
/// `(float) "abc" === 0.0`).
fn parse_number(raw: &str) -> Option<Number> {
    let s = xml::php_trim(raw);
    let b = s.as_bytes();
    let digits = b.strip_prefix(b"-").unwrap_or(b);
    if !digits.is_empty() && digits.len() <= 18 && digits.iter().all(u8::is_ascii_digit) {
        let mut n: i64 = 0;
        for d in digits {
            n = n * 10 + i64::from(d - b'0');
        }
        return Some(Number::Int(if digits.len() < b.len() { -n } else { n }));
    }
    // Reject Rust-only spellings such as "inf" and "NaN".
    let first = *b.first()?;
    if !(first.is_ascii_digit() || matches!(first, b'.' | b'-' | b'+')) {
        return None;
    }
    let f: f64 = s.parse().ok()?;
    f.is_finite().then_some(Number::Float(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns() {
        assert_eq!(column_from_ref("A1"), Some(0));
        assert_eq!(column_from_ref("Z9"), Some(25));
        assert_eq!(column_from_ref("AA1"), Some(26));
        assert_eq!(column_from_ref("xfd1"), Some(16_383));
        assert_eq!(column_from_ref("12"), None);
        assert_eq!(column_from_ref("ABCD1"), Some(usize::MAX));
    }

    #[test]
    fn numbers() {
        let p = |s| parse_number(s).map(Number::as_f64);
        assert_eq!(p("1500"), Some(1500.0));
        assert_eq!(p("-42"), Some(-42.0));
        assert_eq!(p(" 7 "), Some(7.0));
        assert_eq!(p("1.5E-3"), Some(0.0015));
        assert_eq!(p("1E+20"), Some(1e20));
        assert_eq!(p("abc"), None);
        assert_eq!(p("inf"), None);
        assert_eq!(p("-inf"), None);
        assert_eq!(p(""), None);
        assert!(matches!(parse_number("123"), Some(Number::Int(123))));
        assert!(matches!(parse_number("-0"), Some(Number::Int(0))));
        // 19+ digits go through f64, like PHP.
        assert!(matches!(
            parse_number("12345678901234567890"),
            Some(Number::Float(_))
        ));
    }

    #[test]
    fn placing_cells() {
        let mut cells = vec![];
        place(&mut cells, 2, CellValue::Int(3));
        place(&mut cells, 0, CellValue::Int(1));
        assert_eq!(
            cells,
            vec![CellValue::Int(1), CellValue::Empty, CellValue::Int(3)]
        );
    }
}
