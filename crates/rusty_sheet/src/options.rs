//! Parse and validate the `$options` array given to `Reader::open()`.

use ext_php_rs::exception::PhpResult;
use ext_php_rs::types::{ArrayKey, ZendHashTable, Zval};

use crate::exceptions::invalid_option;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateMode {
    /// Excel serial numbers (int for whole days), like Laravel Excel.
    Serial,
    /// `DateTimeImmutable` / `DateInterval` objects, like OpenSpout.
    DateTime,
    /// Display strings rendered through the cell's number format (Phase 3).
    Formatted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnError {
    Throw,
    SkipRow,
}

/// `formulas_without_cached_value`: what a formula the writing application never
/// calculated gives with `calculated_formulas`. Ignored without it (formulas are
/// then always returned as text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UncachedFormula {
    /// `null`, as OpenSpout 5.15+ (the default).
    Null,
    /// A `ReaderException` naming the cell.
    Throw,
    /// The formula text, `"=…"`.
    Formula,
}

/// `$options['csv']`. Unset fields use the format's defaults.
#[derive(Debug, Clone, Default)]
pub struct CsvSettings {
    pub delimiter: Option<u8>,
    pub enclosure: Option<u8>,
    /// `Some(None)`: explicitly no escape character.
    pub escape: Option<Option<u8>>,
    pub encoding: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReaderOptions {
    pub format: Option<String>,
    pub dates: DateMode,
    pub timezone: Option<String>,
    pub calculated_formulas: bool,
    pub formulas_without_cached_value: UncachedFormula,
    pub preserve_empty_rows: bool,
    /// `empty_cells => null`: blank cells inside a row become `null` instead of `''`.
    pub empty_cells_null: bool,
    pub on_error: OnError,
    pub csv: CsvSettings,
}

impl Default for ReaderOptions {
    fn default() -> Self {
        ReaderOptions {
            format: None,
            dates: DateMode::Serial,
            timezone: None,
            calculated_formulas: false,
            formulas_without_cached_value: UncachedFormula::Null,
            preserve_empty_rows: false,
            empty_cells_null: false,
            on_error: OnError::Throw,
            csv: CsvSettings::default(),
        }
    }
}

const KNOWN: &[&str] = &[
    "format",
    "dates",
    "timezone",
    "calculated_formulas",
    "formulas_without_cached_value",
    "preserve_empty_rows",
    "empty_cells",
    "on_error",
    "csv",
];

impl ReaderOptions {
    pub fn parse(arr: Option<&ZendHashTable>) -> PhpResult<Self> {
        let mut o = ReaderOptions::default();
        let Some(arr) = arr else {
            return Ok(o);
        };
        for (key, value) in arr.iter() {
            if let ArrayKey::Long(i) = key {
                return Err(invalid_option(
                    &i.to_string(),
                    "option keys must be strings",
                ));
            }
            let key = key.to_string();
            let value = value.dereference();
            match key.as_str() {
                "format" => o.format = nullable_string(&key, value)?.map(|s| s.to_lowercase()),
                "dates" => {
                    o.dates = match string(&key, value)?.as_str() {
                        "serial" => DateMode::Serial,
                        "datetime" => DateMode::DateTime,
                        "formatted" => DateMode::Formatted,
                        other => {
                            return Err(invalid_option(
                                &key,
                                &format!(
                                    "expected 'serial', 'datetime' or 'formatted', got '{other}'"
                                ),
                            ));
                        }
                    }
                }
                "timezone" => o.timezone = nullable_string(&key, value)?,
                "calculated_formulas" => o.calculated_formulas = boolean(&key, value)?,
                "formulas_without_cached_value" => {
                    o.formulas_without_cached_value = match string(&key, value)?.as_str() {
                        "null" => UncachedFormula::Null,
                        "throw" => UncachedFormula::Throw,
                        "formula" => UncachedFormula::Formula,
                        other => {
                            return Err(invalid_option(
                                &key,
                                &format!("expected 'null', 'throw' or 'formula', got '{other}'"),
                            ));
                        }
                    }
                }
                "preserve_empty_rows" => o.preserve_empty_rows = boolean(&key, value)?,
                "empty_cells" => {
                    o.empty_cells_null = match nullable_string(&key, value) {
                        Ok(None) => true,
                        Ok(Some(s)) if s.is_empty() => false,
                        _ => return Err(invalid_option(&key, "expected '' or null")),
                    }
                }
                "on_error" => {
                    o.on_error = match string(&key, value)?.as_str() {
                        "throw" => OnError::Throw,
                        "skip_row" => OnError::SkipRow,
                        other => {
                            return Err(invalid_option(
                                &key,
                                &format!("expected 'throw' or 'skip_row', got '{other}'"),
                            ));
                        }
                    }
                }
                "csv" if value.is_null() => {}
                "csv" => {
                    o.csv = parse_csv(
                        value
                            .array()
                            .filter(|_| value.is_array())
                            .ok_or_else(|| invalid_option("csv", "expected an array"))?,
                    )?
                }
                other => {
                    return Err(invalid_option(
                        other,
                        &format!("unknown option; known options: {}", KNOWN.join(", ")),
                    ));
                }
            }
        }
        if o.timezone.is_some() && o.dates != DateMode::DateTime {
            return Err(invalid_option(
                "timezone",
                "only applies with dates => 'datetime' (convert datetimes to serials yourself to combine both)",
            ));
        }
        Ok(o)
    }
}

fn parse_csv(arr: &ZendHashTable) -> PhpResult<CsvSettings> {
    let mut c = CsvSettings::default();
    for (key, value) in arr.iter() {
        let key = key.to_string();
        let name = format!("csv.{key}");
        let value = value.dereference();
        match key.as_str() {
            "delimiter" => c.delimiter = Some(single_byte(&name, &string(&name, value)?)?),
            "enclosure" => c.enclosure = Some(single_byte(&name, &string(&name, value)?)?),
            "escape" => {
                let s = string(&name, value)?;
                c.escape = Some(if s.is_empty() {
                    None
                } else {
                    Some(single_byte(&name, &s)?)
                });
            }
            "encoding" => c.encoding = Some(string(&name, value)?),
            _ => {
                return Err(invalid_option(
                    &name,
                    "unknown option; known: delimiter, enclosure, escape, encoding",
                ));
            }
        }
    }
    Ok(c)
}

/// Delimiter, enclosure and escape are single bytes, as in PHP's fgetcsv().
fn single_byte(key: &str, s: &str) -> PhpResult<u8> {
    match s.as_bytes() {
        [b] => Ok(*b),
        _ => Err(invalid_option(key, "must be a single character")),
    }
}

fn string(key: &str, v: &Zval) -> PhpResult<String> {
    v.string()
        .filter(|_| v.is_string())
        .ok_or_else(|| invalid_option(key, "expected a string"))
}

fn nullable_string(key: &str, v: &Zval) -> PhpResult<Option<String>> {
    if v.is_null() {
        Ok(None)
    } else {
        string(key, v).map(Some)
    }
}

fn boolean(key: &str, v: &Zval) -> PhpResult<bool> {
    v.bool()
        .ok_or_else(|| invalid_option(key, "expected a bool"))
}
