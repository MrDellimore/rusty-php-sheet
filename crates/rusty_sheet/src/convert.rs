//! `CellValue` → PHP values, according to the reader options.
//!
//! | CellValue            | dates = serial (default)         | dates = datetime              |
//! |----------------------|----------------------------------|-------------------------------|
//! | Date (point in time) | int/float serial (sheet-stream)  | DateTimeImmutable (+timezone) |
//! | Date (duration)      | day fraction                     | DateInterval                  |
//! | Date (any)           | dates = formatted: the display string OpenSpout renders (SHOULD_FORMAT_DATES) |
//! | Formula              | "=FORMULA", or the cached value with `calculated_formulas` |
//! | Empty (mid-row gap)  | ''                                                               |

use std::collections::HashMap;

use ext_php_rs::boxed::ZBox;
use ext_php_rs::convert::IntoZval;
use ext_php_rs::exception::PhpResult;
use ext_php_rs::types::{ZendHashTable, ZendObject, ZendStr, Zval};
use ext_php_rs::zend::{ClassEntry, Function};

use rusty_sheet_core::xlsx::dates::{
    SerialValue, duration_parts, normalised_serial, serial_to_parts,
};
use rusty_sheet_core::xlsx::php_date::{format_duration, to_php_date_format};
use rusty_sheet_core::xlsx::{SharedStrings, Styles, php_date};
use rusty_sheet_core::{CellValue, DateKind, NumFmtId};

use crate::exceptions::reader_error;
use crate::options::{DateMode, ReaderOptions};

/// Excel serial of 1970-01-01.
const UNIX_EPOCH_SERIAL: i64 = 25_569;

/// Converts cells for one `nextRows()` call. PHP callables are looked up lazily,
/// so batches without dates pay nothing for them.
pub struct Converter<'a> {
    opts: &'a ReaderOptions,
    shared: &'a SharedStrings,
    styles: &'a Styles,
    /// Excel format code → PHP date() format, per number format (kept across batches).
    php_formats: &'a mut HashMap<u32, String>,
    date_create: Option<Function>,
    timezone: Option<Zval>,
}

impl<'a> Converter<'a> {
    pub fn new(
        opts: &'a ReaderOptions,
        shared: &'a SharedStrings,
        styles: &'a Styles,
        php_formats: &'a mut HashMap<u32, String>,
    ) -> Self {
        Converter {
            opts,
            shared,
            styles,
            php_formats,
            date_create: None,
            timezone: None,
        }
    }

    pub fn row(&mut self, cells: &[CellValue]) -> PhpResult<ZBox<ZendHashTable>> {
        let mut arr = ZendHashTable::with_capacity(cells.len() as u32);
        for cell in cells {
            arr.push(self.value(cell)?)?;
        }
        Ok(arr)
    }

    fn value(&mut self, v: &CellValue) -> PhpResult<Zval> {
        let mut z = Zval::new();
        match v {
            CellValue::Empty => z.set_interned_string("", false)?,
            CellValue::String(s) => z.set_string(s, false)?,
            CellValue::Bytes(b) => z.set_zend_string(ZendStr::new(b, false)),
            CellValue::SharedString(i) => z.set_string(self.shared.get(*i).unwrap_or(""), false)?,
            CellValue::Int(i) => z.set_long(*i),
            CellValue::Float(f) => z.set_double(*f),
            CellValue::Bool(b) => z.set_bool(*b),
            CellValue::Null => z.set_null(),
            CellValue::Date {
                serial,
                kind: DateKind::DateTime,
                fmt,
            } => match self.opts.dates {
                DateMode::Serial => set_serial(&mut z, normalised_serial(*serial)),
                DateMode::DateTime => return self.datetime(&wall_clock(*serial)),
                DateMode::Formatted => return self.formatted_date(*serial, *fmt),
            },
            CellValue::Date {
                serial,
                kind: DateKind::Duration,
                fmt,
            } => {
                let parts = duration_parts(*serial);
                match self.opts.dates {
                    DateMode::Serial => set_serial(&mut z, parts.to_serial()),
                    DateMode::DateTime => {
                        return date_interval(&parts.interval_spec(), parts.negative);
                    }
                    DateMode::Formatted => z.set_string(
                        &format_duration(self.styles.format_code(*fmt), &parts),
                        false,
                    )?,
                }
            }
            // OpenSpout returns the raw ISO text when formatting dates.
            CellValue::IsoDate(text) if self.opts.dates == DateMode::Formatted => {
                z.set_string(text, false)?
            }
            CellValue::IsoDate(text) => return self.iso_date(text),
            CellValue::Formula { formula, cached } => {
                if self.opts.calculated_formulas {
                    return match cached {
                        Some(c) => self.value(c),
                        None => Ok(Zval::null()),
                    };
                }
                z.set_string(&format!("={formula}"), false)?;
            }
        }
        Ok(z)
    }

    /// The display string for a date-formatted cell: rendered in Rust when the PHP
    /// format only uses characters we implement, otherwise by PHP itself.
    fn formatted_date(&mut self, serial: f64, fmt: NumFmtId) -> PhpResult<Zval> {
        let styles = self.styles;
        let php_format = self
            .php_formats
            .entry(fmt.0)
            .or_insert_with(|| to_php_date_format(styles.format_code(fmt)))
            .clone();
        let parts = serial_to_parts(serial);
        if let Some(text) = php_date::format(&parts, &php_format) {
            let mut z = Zval::new();
            z.set_string(&text, false)?;
            return Ok(z);
        }
        let dt = self.datetime(&wall_clock(serial))?;
        match dt.object() {
            Some(obj) => obj
                .try_call_method("format", vec![&php_format])
                .map_err(|e| reader_error(format!("DateTimeImmutable::format: {e}"))),
            None => Ok(Zval::null()),
        }
    }

    /// `date_create_immutable($text)` in PHP's default timezone (as OpenSpout does),
    /// then moved to the configured timezone. Unparseable text gives `null`.
    fn datetime(&mut self, text: &str) -> PhpResult<Zval> {
        if self.date_create.is_none() {
            self.date_create = Some(
                Function::try_from_function("date_create_immutable")
                    .ok_or_else(|| reader_error("date_create_immutable() is unavailable"))?,
            );
        }
        let f = self.date_create.as_ref().expect("set above");
        let dt = f
            .try_call(vec![&text])
            .map_err(|e| reader_error(format!("creating DateTimeImmutable: {e}")))?;
        if !dt.is_object() {
            return Ok(Zval::null());
        }
        let Some(tz_name) = &self.opts.timezone else {
            return Ok(dt);
        };
        if self.timezone.is_none() {
            self.timezone = Some(new_object("DateTimeZone", tz_name.as_str())?);
        }
        let tz = self.timezone.as_ref().expect("set above");
        dt.object()
            .expect("checked")
            .try_call_method("setTimezone", vec![tz])
            .map_err(|e| reader_error(format!("setTimezone: {e}")))
    }

    /// A `t="d"` cell: OpenSpout parses it with `new DateTimeImmutable($text)`.
    fn iso_date(&mut self, text: &str) -> PhpResult<Zval> {
        let dt = self.datetime(text)?;
        if self.opts.dates != DateMode::Serial || !dt.is_object() {
            return Ok(dt);
        }
        // Serial mode: sheet-stream's dateToSerial (wall clock, without its leap-day bug).
        let obj = dt.object().expect("checked");
        let call = |name: &str| -> PhpResult<i64> {
            obj.try_call_method(name, vec![])
                .ok()
                .and_then(|z| z.long())
                .ok_or_else(|| reader_error(format!("DateTimeImmutable::{name}() failed")))
        };
        let local = call("getTimestamp")? + call("getOffset")?;
        let days = local.div_euclid(86_400) + UNIX_EPOCH_SERIAL;
        let secs = local.rem_euclid(86_400);
        let mut z = Zval::new();
        set_serial(
            &mut z,
            if secs == 0 {
                SerialValue::Int(days)
            } else {
                SerialValue::Float(days as f64 + secs as f64 / 86_400.0)
            },
        );
        Ok(z)
    }
}

/// "Y-m-d H:i:s" for a serial, with OpenSpout's rounding.
fn wall_clock(serial: f64) -> String {
    let p = serial_to_parts(serial);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        p.year, p.month, p.day, p.hour, p.minute, p.second
    )
}

fn set_serial(z: &mut Zval, v: SerialValue) {
    match v {
        SerialValue::Int(i) => z.set_long(i),
        SerialValue::Float(f) => z.set_double(f),
    }
}

/// `new DateInterval($spec)`, inverted for negative durations (OpenSpout parity).
fn date_interval(spec: &str, negative: bool) -> PhpResult<Zval> {
    let mut obj = construct("DateInterval", spec)?;
    if negative {
        obj.set_property("invert", 1i64)?;
    }
    Ok(obj.into_zval(false)?)
}

fn new_object(class: &str, arg: &str) -> PhpResult<Zval> {
    Ok(construct(class, arg)?.into_zval(false)?)
}

/// `new $class($arg)`. If the constructor throws, the PHP exception propagates.
fn construct(class: &str, arg: &str) -> PhpResult<ZBox<ZendObject>> {
    let ce = ClassEntry::try_find(class)
        .ok_or_else(|| reader_error(format!("class {class} is unavailable")))?;
    let obj = ZendObject::new(ce);
    obj.try_call_method("__construct", vec![&arg])
        .map_err(|e| reader_error(format!("new {class}('{arg}'): {e}")))?;
    Ok(obj)
}
