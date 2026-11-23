//! `RustySheet\Writer` and `RustySheet\Cell`: streaming XLSX/CSV writer for PHP,
//! producing the same files as OpenSpout's writers (see `rusty_sheet_core::writer`).

use std::path::Path;

use ext_php_rs::exception::{PhpException, PhpResult};
use ext_php_rs::flags::ClassFlags;
use ext_php_rs::prelude::*;
use ext_php_rs::types::{ArrayKey, ZendHashTable, ZendObject, Zval};
use ext_php_rs::zend::ClassEntry;

use rusty_sheet_core::Error;
use rusty_sheet_core::writer::{
    Border, BorderLine, BorderPart, BorderSide, BorderWidth, Cell as WCell, CellData, CsvWriter,
    CsvWriterOptions, Link, Style, XlsxWriter, excel_serial, interval_serial,
};

use crate::exceptions::{WriterException, invalid_option, unsupported};

fn writer_error(message: impl Into<String>) -> PhpException {
    PhpException::from_class::<WriterException>(message.into())
}

fn from_core(err: Error, path: &str) -> PhpException {
    match err {
        Error::InvalidData { message, .. } => writer_error(format!("{path}: {message}")),
        Error::InvalidArgument(message) => {
            PhpException::new(message, 0, ext_php_rs::zend::ce::value_error())
        }
        other => writer_error(format!("{path}: {other}")),
    }
}

/// A value converted from PHP, in the forms both writers need.
#[derive(Debug, Clone)]
enum Value {
    Empty,
    /// Raw PHP string bytes (may be invalid UTF-8).
    Text(Vec<u8>),
    /// Raw bytes without the leading `=` (may be invalid UTF-8).
    Formula(Vec<u8>),
    Int(i64),
    /// `php` is PHP's `(string)` of the float (CSV output).
    Float {
        v: f64,
        php: String,
    },
    Bool(bool),
    /// `atom` is `DATE_ATOM` (CSV output).
    Date {
        serial: f64,
        has_time: bool,
        atom: String,
    },
    /// `iso` is OpenSpout's CSV form of a DateInterval.
    Interval {
        serial: f64,
        iso: String,
    },
}

/// `RustySheet\Cell`: a value with its own style, or text that must not become a
/// formula. Created with `Writer::cell()` / `Writer::text()`.
#[php_class]
#[php(name = "RustySheet\\Cell")]
#[php(flags = ClassFlags::Final)]
pub struct PhpCell {
    value: Value,
    style: Option<usize>,
    /// `(target, tooltip)` from `Writer::link()`.
    link: Option<(String, Option<String>)>,
}

/// What a writer is writing.
enum Target {
    Xlsx(Box<XlsxState>),
    Csv { writer: CsvWriter, sheets: u32 },
}

struct XlsxState {
    writer: XlsxWriter,
    date_format: String,
    datetime_format: String,
    /// Lazily created styles for dates without a formatted column style.
    date_style: Option<usize>,
    datetime_style: Option<usize>,
    /// Format code of each style handle (dates use a column style only if it has one).
    style_formats: Vec<Option<String>>,
}

#[php_class]
#[php(name = "RustySheet\\Writer")]
#[php(flags = ClassFlags::Final)]
pub struct Writer {
    path: String,
    /// `None` once closed.
    target: Option<Target>,
}

#[php_impl]
impl Writer {
    /// Create an XLSX, CSV or TSV file (format from `$options['format']` or the extension).
    pub fn create(path: String, options: Option<&ZendHashTable>) -> PhpResult<Self> {
        let o = WriterOptions::parse(options)?;
        let format = o.format.clone().unwrap_or_else(|| {
            Path::new(&path)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });
        let target = match format.as_str() {
            "xlsx" => Target::Xlsx(Box::new(XlsxState {
                writer: XlsxWriter::create(Path::new(&path)).map_err(|e| from_core(e, &path))?,
                date_format: o.date_format,
                datetime_format: o.datetime_format,
                date_style: None,
                datetime_style: None,
                style_formats: Vec::new(),
            })),
            "csv" | "tsv" => {
                let mut csv = o.csv;
                if format == "tsv" && !o.delimiter_set {
                    csv.delimiter = b'\t';
                }
                Target::Csv {
                    writer: CsvWriter::create(Path::new(&path), csv)
                        .map_err(|e| from_core(e, &path))?,
                    sheets: 0,
                }
            }
            other => {
                return Err(unsupported(format!(
                    "{path}: format '{other}' can't be written by rusty_sheet"
                )));
            }
        };
        Ok(Writer {
            path,
            target: Some(target),
        })
    }

    /// The first call names the default sheet; later calls start a new one (XLSX only).
    pub fn add_sheet(&mut self, name: Option<String>) -> PhpResult<()> {
        let path = self.path.clone();
        match self.target_mut()? {
            Target::Xlsx(x) => x
                .writer
                .add_sheet(name.as_deref())
                .map_err(|e| from_core(e, &path)),
            Target::Csv { sheets, .. } => {
                *sheets += 1;
                if *sheets > 1 {
                    return Err(unsupported(
                        "The CSV format does not support multiple sheets.",
                    ));
                }
                Ok(())
            }
        }
    }

    /// Width of 1-based columns, before the sheet's first row. Ignored for CSV.
    pub fn set_column_width(&mut self, width: f64, columns: Vec<i64>) -> PhpResult<()> {
        let path = self.path.clone();
        if let Target::Xlsx(x) = self.target_mut()? {
            let cols: Vec<u32> = columns
                .iter()
                .map(|&c| u32::try_from(c).ok().filter(|&c| c > 0))
                .collect::<Option<_>>()
                .ok_or_else(|| invalid_option("columns", "column numbers are 1-based"))?;
            x.writer
                .set_column_width(width, &cols)
                .map_err(|e| from_core(e, &path))?;
        }
        Ok(())
    }

    /// Freeze the top `$rows` rows and the left `$columns` columns. Ignored for CSV.
    #[php(defaults(columns = 0))]
    pub fn freeze_panes(&mut self, rows: i64, columns: i64) -> PhpResult<()> {
        let path = self.path.clone();
        if let Target::Xlsx(x) = self.target_mut()? {
            let rows = u32::try_from(rows).map_err(|_| invalid_option("rows", "must be >= 0"))?;
            let cols =
                u32::try_from(columns).map_err(|_| invalid_option("columns", "must be >= 0"))?;
            x.writer
                .freeze(rows, cols)
                .map_err(|e| from_core(e, &path))?;
        }
        Ok(())
    }

    /// Show or hide gridlines. Ignored for CSV.
    pub fn show_grid_lines(&mut self, show: bool) -> PhpResult<()> {
        let path = self.path.clone();
        if let Target::Xlsx(x) = self.target_mut()? {
            x.writer
                .show_grid_lines(show)
                .map_err(|e| from_core(e, &path))?;
        }
        Ok(())
    }

    /// Merge a range of the current sheet (`'A1:H1'`), any time before the sheet ends.
    /// Ignored for CSV.
    pub fn merge_cells(&mut self, range: String) -> PhpResult<()> {
        let path = self.path.clone();
        if let Target::Xlsx(x) = self.target_mut()? {
            x.writer
                .merge_cells(&range)
                .map_err(|e| from_core(e, &path))?;
        }
        Ok(())
    }

    /// Put an autofilter on a range of the current sheet, before its first row.
    /// Ignored for CSV.
    pub fn set_auto_filter(&mut self, range: String) -> PhpResult<()> {
        let path = self.path.clone();
        if let Target::Xlsx(x) = self.target_mut()? {
            x.writer
                .set_auto_filter(&range)
                .map_err(|e| from_core(e, &path))?;
        }
        Ok(())
    }

    /// Register a style and return its id. Every call is a distinct style.
    pub fn add_style(&mut self, style: &ZendHashTable) -> PhpResult<i64> {
        let style = parse_style(style)?;
        match self.target_mut()? {
            Target::Xlsx(x) => {
                x.style_formats.push(style.format.clone());
                Ok(x.writer.add_style(style) as i64)
            }
            // CSV has no styles; ids are still handed out so callers needn't care.
            Target::Csv { .. } => Ok(0),
        }
    }

    /// Write one row. Values: string|int|float|bool|null|DateTimeInterface|DateInterval|Cell.
    /// A string starting with "=" is a formula unless wrapped with `Writer::text()`.
    pub fn add_row(
        &mut self,
        values: &ZendHashTable,
        row_style: Option<i64>,
        column_styles: Option<&ZendHashTable>,
        height: Option<f64>,
    ) -> PhpResult<()> {
        let path = self.path.clone();
        let target = self.target_mut()?;
        let is_csv = matches!(target, Target::Csv { .. });
        let converted: Vec<Converted> = values
            .values()
            .map(|z| to_value(z, is_csv))
            .collect::<PhpResult<_>>()?;

        match target {
            Target::Csv { writer, .. } => {
                let fields: Vec<Vec<u8>> = converted.iter().map(|c| csv_field(&c.value)).collect();
                writer.add_row(&fields).map_err(|e| from_core(e, &path))
            }
            Target::Xlsx(x) => {
                let row_style = row_style.map(style_handle).transpose()?;
                let column_styles = parse_column_styles(column_styles)?;
                // Check every style id first, so a bad one fails before anything is written.
                let known = x.style_formats.len();
                let unknown = row_style
                    .into_iter()
                    .chain(column_styles.iter().flatten().copied())
                    .chain(converted.iter().filter_map(|c| c.own_style))
                    .find(|&h| h >= known);
                if let Some(h) = unknown {
                    return Err(writer_error(format!("{path}: unknown style id {h}")));
                }

                let mut cells = Vec::with_capacity(converted.len());
                // Strings must outlive the borrows in `CellData`, so convert them first.
                // OpenSpout's htmlspecialchars() turns invalid UTF-8 into ''.
                let texts: Vec<String> = converted
                    .iter()
                    .filter_map(|c| match &c.value {
                        Value::Text(b) | Value::Formula(b) => {
                            Some(String::from_utf8(b.clone()).unwrap_or_default())
                        }
                        _ => None,
                    })
                    .collect();
                let mut text_iter = texts.iter();
                for (i, c) in converted.iter().enumerate() {
                    let (v, own_style, is_native) = (&c.value, &c.own_style, &c.native);
                    let column_style = column_styles.get(i).copied().flatten().or(row_style);
                    let style = if *is_native { *own_style } else { column_style };
                    // OpenSpout checks the raw bytes for "\n", even when it writes ''.
                    let multiline = matches!(v, Value::Text(b) if b.contains(&b'\n'));
                    let (data, style) = match v {
                        Value::Empty => (CellData::Empty, style),
                        Value::Text(_) => (
                            CellData::Str(text_iter.next().expect("collected above")),
                            style,
                        ),
                        Value::Formula(_) => (
                            CellData::Formula(text_iter.next().expect("collected above")),
                            style,
                        ),
                        Value::Int(i) => (CellData::Int(*i), style),
                        Value::Float { v, .. } => (CellData::Float(*v), style),
                        Value::Bool(b) => (CellData::Bool(*b), style),
                        Value::Interval { serial, .. } => (CellData::Float(*serial), style),
                        Value::Date {
                            serial, has_time, ..
                        } => {
                            let style = if *is_native {
                                style
                            } else {
                                Some(x.date_style_for(column_style, *has_time))
                            };
                            (CellData::Float(*serial), style)
                        }
                    };
                    cells.push(WCell {
                        data,
                        style,
                        multiline,
                        link: c.link.as_ref().map(|(target, tooltip)| Link {
                            target,
                            tooltip: tooltip.as_deref(),
                        }),
                    });
                }
                x.writer
                    .add_row(&cells, height)
                    .map_err(|e| from_core(e, &path))
            }
        }
    }

    /// A value with its own style (OpenSpout's native-Cell passthrough).
    pub fn cell(value: &Zval, style: Option<i64>) -> PhpResult<PhpCell> {
        // The target format isn't known here, so keep the CSV forms too.
        let c = to_value(value, true)?;
        Ok(PhpCell {
            value: c.value,
            style: style.map(style_handle).transpose()?,
            link: None,
        })
    }

    /// Text with a hyperlink: a URL, `#'Sheet name'!A1` for a place in the workbook,
    /// or PhpSpreadsheet's `sheet://'Sheet name'!A1`. CSV writes the text only.
    pub fn link(
        text: String,
        target: String,
        style: Option<i64>,
        tooltip: Option<String>,
    ) -> PhpResult<PhpCell> {
        if target.is_empty() || target == "#" || target == "sheet://" {
            return Err(invalid_option("target", "a link needs a target"));
        }
        Ok(PhpCell {
            value: Value::Text(text.into_bytes()),
            style: style.map(style_handle).transpose()?,
            link: Some((target, tooltip)),
        })
    }

    /// Text that is never treated as a formula.
    pub fn text(value: &Zval, style: Option<i64>) -> PhpResult<PhpCell> {
        let bytes = value
            .zend_str()
            .map(|s| s.as_bytes().to_vec())
            .ok_or_else(|| invalid_option("value", "expected a string"))?;
        // Even '' is written as a (blank) string, like OpenSpout's StringCell.
        Ok(PhpCell {
            value: Value::Text(bytes),
            style: style.map(style_handle).transpose()?,
            link: None,
        })
    }

    /// Finish the file. Without it, the file is incomplete.
    pub fn close(&mut self) -> PhpResult<()> {
        let path = self.path.clone();
        match self.target.take() {
            Some(Target::Xlsx(x)) => x.writer.close().map_err(|e| from_core(e, &path)),
            Some(Target::Csv { writer, .. }) => writer.close().map_err(|e| from_core(e, &path)),
            None => Ok(()),
        }
    }
}

impl Writer {
    fn target_mut(&mut self) -> PhpResult<&mut Target> {
        self.target
            .as_mut()
            .ok_or_else(|| writer_error("the writer is closed"))
    }
}

impl XlsxState {
    /// OpenSpoutWriter::resolveDateStyle: the column style if it has a format,
    /// otherwise one shared date or date-time style.
    fn date_style_for(&mut self, column_style: Option<usize>, has_time: bool) -> usize {
        if let Some(h) = column_style
            && self.style_formats.get(h).is_some_and(Option::is_some)
        {
            return h;
        }
        let (slot, code) = if has_time {
            (&mut self.datetime_style, self.datetime_format.clone())
        } else {
            (&mut self.date_style, self.date_format.clone())
        };
        if let Some(h) = *slot {
            return h;
        }
        let h = self.writer.add_style(Style {
            format: Some(code.clone()),
            ..Style::default()
        });
        self.style_formats.push(Some(code));
        *slot = Some(h);
        h
    }
}

fn style_handle(id: i64) -> PhpResult<usize> {
    usize::try_from(id).map_err(|_| invalid_option("style", "unknown style id"))
}

fn parse_column_styles(arr: Option<&ZendHashTable>) -> PhpResult<Vec<Option<usize>>> {
    let mut out: Vec<Option<usize>> = Vec::new();
    let Some(arr) = arr else {
        return Ok(out);
    };
    for (key, value) in arr.iter() {
        let ArrayKey::Long(col) = key else {
            return Err(invalid_option(
                "columnStyles",
                "keys must be 0-based column indexes",
            ));
        };
        let col = usize::try_from(col)
            .map_err(|_| invalid_option("columnStyles", "keys must be >= 0"))?;
        let id = value
            .long()
            .ok_or_else(|| invalid_option("columnStyles", "values must be style ids"))?;
        if out.len() <= col {
            out.resize(col + 1, None);
        }
        out[col] = Some(style_handle(id)?);
    }
    Ok(out)
}

/// The CSV text of a value, as OpenSpout's CSV writer produces it.
fn csv_field(v: &Value) -> Vec<u8> {
    match v {
        Value::Empty => Vec::new(),
        Value::Text(b) => b.clone(),
        Value::Formula(f) => [b"=".as_slice(), f].concat(),
        Value::Int(i) => i.to_string().into_bytes(),
        Value::Float { php, .. } => php.clone().into_bytes(),
        Value::Bool(b) => {
            if *b {
                b"1".to_vec()
            } else {
                b"0".to_vec()
            }
        }
        Value::Date { atom, .. } => atom.clone().into_bytes(),
        Value::Interval { iso, .. } => iso.clone().into_bytes(),
    }
}

/// A PHP value converted for writing.
struct Converted {
    value: Value,
    /// A native `RustySheet\Cell`'s own style.
    own_style: Option<usize>,
    /// It was a native `RustySheet\Cell`, whose style is used as is.
    native: bool,
    link: Option<(String, Option<String>)>,
}

impl From<Value> for Converted {
    fn from(value: Value) -> Self {
        Converted {
            value,
            own_style: None,
            native: false,
            link: None,
        }
    }
}

/// Convert a PHP value.
fn to_value(z: &Zval, for_csv: bool) -> PhpResult<Converted> {
    let z = z.dereference();
    if z.is_null() {
        return Ok(Value::Empty.into());
    }
    if z.is_bool() {
        return Ok(Value::Bool(z.is_true()).into());
    }
    if z.is_long() {
        return Ok(Value::Int(z.long().unwrap_or(0)).into());
    }
    if z.is_double() {
        let v = z.double().unwrap_or(0.0);
        // PHP's own (string) conversion, honouring the `precision` ini setting.
        let php = if for_csv {
            php_strval(z)?
        } else {
            String::new()
        };
        return Ok(Value::Float { v, php }.into());
    }
    if let Some(s) = z.zend_str().filter(|_| z.is_string()) {
        let bytes = s.as_bytes();
        return Ok(match bytes {
            [] => Value::Empty,
            [b'=', rest @ ..] => Value::Formula(rest.to_vec()),
            _ => Value::Text(bytes.to_vec()),
        }
        .into());
    }
    if let Some(obj) = z.object() {
        if let Some(cell) = z.extract::<&PhpCell>() {
            return Ok(Converted {
                value: cell.value.clone(),
                own_style: cell.style,
                native: true,
                link: cell.link.clone(),
            });
        }
        if instance_of(obj, "DateTimeInterface") {
            return Ok(date_value(obj, for_csv)?.into());
        }
        if instance_of(obj, "DateInterval") {
            return Ok(interval_value(obj, for_csv)?.into());
        }
        let class = obj.get_class_name().unwrap_or_default();
        return Err(writer_error(format!(
            "cannot write a value of type {class}"
        )));
    }
    if z.is_array() {
        return Err(writer_error(
            "cannot write an array (rich text runs are not supported by rusty_sheet)",
        ));
    }
    Err(writer_error("cannot write a value of this type"))
}

/// `strval($z)`, exactly as PHP converts it (floats use the `precision` ini setting).
fn php_strval(z: &Zval) -> PhpResult<String> {
    ext_php_rs::zend::Function::try_from_function("strval")
        .and_then(|f| f.try_call(vec![z]).ok())
        .and_then(|r| r.string())
        .ok_or_else(|| writer_error("strval() failed"))
}

fn instance_of(obj: &ZendObject, class: &str) -> bool {
    ClassEntry::try_find(class).is_some_and(|ce| obj.instance_of(ce))
}

fn call_string(obj: &ZendObject, method: &str, arg: &str) -> PhpResult<String> {
    obj.try_call_method(method, vec![&arg])
        .ok()
        .and_then(|z| z.string())
        .ok_or_else(|| writer_error(format!("{method}() failed")))
}

/// DateHelper::toExcel from the wall-clock fields, plus DATE_ATOM for CSV.
fn date_value(obj: &ZendObject, for_csv: bool) -> PhpResult<Value> {
    let fields = call_string(obj, "format", "Y n j G i s")?;
    let n: Vec<i64> = fields.split(' ').map(|p| p.parse().unwrap_or(0)).collect();
    let [y, m, d, h, i, s] = n[..] else {
        return Err(writer_error("unexpected date format"));
    };
    Ok(Value::Date {
        serial: excel_serial(y, m, d, h, i, s),
        has_time: h != 0 || i != 0 || s != 0,
        atom: if for_csv {
            call_string(obj, "format", "Y-m-d\\TH:i:sP")?
        } else {
            String::new()
        },
    })
}

fn interval_value(obj: &ZendObject, for_csv: bool) -> PhpResult<Value> {
    let prop = |name: &str| obj.get_property::<i64>(name).unwrap_or(0);
    Ok(Value::Interval {
        serial: interval_serial(
            prop("y"),
            prop("m"),
            prop("d"),
            prop("h"),
            prop("i"),
            prop("s"),
            prop("invert") == 1,
        ),
        iso: if for_csv {
            call_string(obj, "format", "P%yY%mM%dDT%hH%iM%sS%fF")?
        } else {
            String::new()
        },
    })
}

/// A style array: bold, italic, underline, strikethrough, size, color, font,
/// background, format, align, valign, wrap, rotation, shrink, border.
fn parse_style(arr: &ZendHashTable) -> PhpResult<Style> {
    let mut st = Style::default();
    for (key, value) in arr.iter() {
        let key = key.to_string();
        let v = value.dereference();
        let name = format!("style.{key}");
        let boolean = || {
            v.bool()
                .filter(|_| v.is_bool())
                .ok_or_else(|| invalid_option(&name, "expected a bool"))
        };
        let int = || {
            v.long()
                .filter(|_| v.is_long())
                .ok_or_else(|| invalid_option(&name, "expected an int"))
        };
        let string = || {
            v.string()
                .filter(|_| v.is_string())
                .ok_or_else(|| invalid_option(&name, "expected a string"))
        };
        match key.as_str() {
            "bold" => st.bold = boolean()?,
            "italic" => st.italic = boolean()?,
            "underline" => st.underline = boolean()?,
            "strikethrough" => st.strikethrough = boolean()?,
            "size" => st.font_size = int()?,
            "color" => st.font_color = string()?,
            "font" => st.font_name = string()?,
            "background" => st.background = Some(string()?),
            "format" => st.format = Some(string()?),
            "align" => {
                st.horizontal = Some(enum_value(
                    &name,
                    string()?,
                    &["left", "right", "center", "justify"],
                )?)
            }
            "valign" => {
                st.vertical = Some(enum_value(
                    &name,
                    string()?,
                    &[
                        "auto",
                        "baseline",
                        "bottom",
                        "center",
                        "distributed",
                        "justify",
                        "top",
                    ],
                )?)
            }
            "wrap" => st.wrap_text = Some(boolean()?),
            "rotation" => st.text_rotation = int()?,
            "shrink" => st.shrink_to_fit = Some(boolean()?),
            "border" => {
                st.border = Some(parse_border(
                    v.array()
                        .filter(|_| v.is_array())
                        .ok_or_else(|| invalid_option(&name, "expected an array"))?,
                )?)
            }
            _ => {
                return Err(invalid_option(
                    &name,
                    "unknown style key; known: bold, italic, underline, strikethrough, size, color, font, background, format, align, valign, wrap, rotation, shrink, border",
                ));
            }
        }
    }
    Ok(st)
}

/// `['top' => ['style' => 'solid', 'width' => 'thin', 'color' => 'RRGGBB'], …]`.
/// Sides: top, right, bottom, left, or `all` (left, right, top, bottom). Each part
/// defaults to OpenSpout's: solid, medium, black. Parts keep the order given, which
/// decides whether two styles share a border entry, as in OpenSpout.
fn parse_border(arr: &ZendHashTable) -> PhpResult<Border> {
    let mut border = Border::default();
    for (key, value) in arr.iter() {
        let side = key.to_string();
        let name = format!("style.border.{side}");
        let sides: &[BorderSide] = match side.as_str() {
            "top" => &[BorderSide::Top],
            "right" => &[BorderSide::Right],
            "bottom" => &[BorderSide::Bottom],
            "left" => &[BorderSide::Left],
            "all" => &BorderSide::ALL,
            _ => {
                return Err(invalid_option(
                    &name,
                    "unknown side; known: top, right, bottom, left, all",
                ));
            }
        };
        let v = value.dereference();
        let spec = v
            .array()
            .filter(|_| v.is_array())
            .ok_or_else(|| invalid_option(&name, "expected an array"))?;
        let mut part = BorderPart::new(BorderSide::Top);
        for (k, pv) in spec.iter() {
            let k = k.to_string();
            let pname = format!("{name}.{k}");
            let pv = pv.dereference();
            let text = pv
                .string()
                .filter(|_| pv.is_string())
                .ok_or_else(|| invalid_option(&pname, "expected a string"))?;
            match k.as_str() {
                "color" => part.color = text,
                "style" => {
                    part.line = match text.as_str() {
                        "none" => BorderLine::None,
                        "solid" => BorderLine::Solid,
                        "dashed" => BorderLine::Dashed,
                        "dotted" => BorderLine::Dotted,
                        "double" => BorderLine::Double,
                        "hair" => BorderLine::Hair,
                        other => {
                            return Err(invalid_option(
                                &pname,
                                &format!(
                                    "'{other}' is not one of: none, solid, dashed, dotted, double, hair"
                                ),
                            ));
                        }
                    }
                }
                "width" => {
                    part.width = match text.as_str() {
                        "thin" => BorderWidth::Thin,
                        "medium" => BorderWidth::Medium,
                        "thick" => BorderWidth::Thick,
                        other => {
                            return Err(invalid_option(
                                &pname,
                                &format!("'{other}' is not one of: thin, medium, thick"),
                            ));
                        }
                    }
                }
                _ => {
                    return Err(invalid_option(
                        &pname,
                        "unknown border key; known: style, width, color",
                    ));
                }
            }
        }
        for &s in sides {
            border = border.with(BorderPart {
                side: s,
                ..part.clone()
            });
        }
    }
    Ok(border)
}

/// OpenSpout's CellAlignment / CellVerticalAlignment enums (`::from()` throws on others).
fn enum_value(name: &str, value: String, allowed: &[&str]) -> PhpResult<String> {
    if allowed.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(invalid_option(
            name,
            &format!("'{value}' is not one of: {}", allowed.join(", ")),
        ))
    }
}

struct WriterOptions {
    format: Option<String>,
    date_format: String,
    datetime_format: String,
    csv: CsvWriterOptions,
    delimiter_set: bool,
}

impl WriterOptions {
    fn parse(arr: Option<&ZendHashTable>) -> PhpResult<Self> {
        let mut o = WriterOptions {
            format: None,
            date_format: "yyyy-mm-dd".into(),
            datetime_format: "yyyy-mm-dd hh:mm:ss".into(),
            csv: CsvWriterOptions::default(),
            delimiter_set: false,
        };
        let Some(arr) = arr else {
            return Ok(o);
        };
        for (key, value) in arr.iter() {
            let key = key.to_string();
            let v = value.dereference();
            let string = |name: &str| {
                v.string()
                    .filter(|_| v.is_string())
                    .ok_or_else(|| invalid_option(name, "expected a string"))
            };
            match key.as_str() {
                "format" => o.format = Some(string("format")?.to_lowercase()),
                "date_format" => o.date_format = string("date_format")?,
                "datetime_format" => o.datetime_format = string("datetime_format")?,
                "csv" => {
                    let csv = v
                        .array()
                        .filter(|_| v.is_array())
                        .ok_or_else(|| invalid_option("csv", "expected an array"))?;
                    for (k, cv) in csv.iter() {
                        let k = k.to_string();
                        let cv = cv.dereference();
                        let name = format!("csv.{k}");
                        let byte = || -> PhpResult<u8> {
                            match cv.str().filter(|_| cv.is_string()).map(str::as_bytes) {
                                Some([b]) => Ok(*b),
                                _ => Err(invalid_option(&name, "must be a single character")),
                            }
                        };
                        match k.as_str() {
                            "delimiter" => {
                                o.csv.delimiter = byte()?;
                                o.delimiter_set = true;
                            }
                            "enclosure" => o.csv.enclosure = byte()?,
                            "bom" => {
                                o.csv.bom = cv
                                    .bool()
                                    .filter(|_| cv.is_bool())
                                    .ok_or_else(|| invalid_option(&name, "expected a bool"))?
                            }
                            "eol" => {
                                o.csv.eol = cv
                                    .zend_str()
                                    .filter(|_| cv.is_string())
                                    .map(|s| s.as_bytes().to_vec())
                                    .ok_or_else(|| invalid_option(&name, "expected a string"))?
                            }
                            _ => {
                                return Err(invalid_option(
                                    &name,
                                    "unknown option; known: delimiter, enclosure, bom, eol",
                                ));
                            }
                        }
                    }
                }
                other => {
                    return Err(invalid_option(
                        other,
                        "unknown option; known: format, date_format, datetime_format, csv",
                    ));
                }
            }
        }
        Ok(o)
    }
}
