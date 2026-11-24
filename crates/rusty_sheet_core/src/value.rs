//! The Rust-side representation of one cell.
//!
//! The core keeps *more* information than PHP ultimately receives (date-ness,
//! formula text alongside cached values). The binding layer picks what to emit
//! based on the reader options (`dates`, `calculated_formulas`); see the table in
//! `docs/implementation-plan.md`, Phase 2.

use crate::xlsx::SharedStrings;

/// A number format id (`numFmtId`), built-in (< 164) or custom.
/// Resolve the format code with `Styles::format_code`; it's needed to render
/// dates in `formatted` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NumFmtId(pub u32);

/// How a date-formatted number should be materialised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateKind {
    /// A point in time (`DateTimeImmutable` in PHP). OpenSpout uses this even for
    /// time-only formats such as `h:mm`; the date part is then 1899-12-30.
    DateTime,
    /// An elapsed-time format such as `[h]:mm:ss` (`DateInterval` in PHP).
    Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    /// A gap inside a row, or a cell with no value. Emitted to PHP as `''`
    /// (OpenSpout parity). The reader trims trailing blanks, so they never reach PHP.
    Empty,
    String(String),
    /// Text that isn't valid UTF-8 (CSV only). PHP strings are bytes, so it's passed
    /// through unchanged, as OpenSpout does for UTF-8 input.
    Bytes(Vec<u8>),
    /// Index into the workbook's shared-string table, resolved lazily to avoid a
    /// copy per cell. Resolve it with `SheetRows::shared_strings()`.
    SharedString(u32),
    /// Numbers with an integral value that fits in i64 (OpenSpout casts these to int).
    Int(i64),
    Float(f64),
    Bool(bool),
    /// Error cells (`#N/A`, `#DIV/0!`, …), unparseable values, out-of-range dates.
    Null,
    /// A number whose number format is a date, time or duration format.
    Date {
        /// Excel serial in the 1900 system. Values from 1904-system workbooks are
        /// shifted by 1462 days on read, so consumers never need to know the system.
        serial: f64,
        kind: DateKind,
        fmt: NumFmtId,
    },
    /// A `t="d"` cell: an ISO 8601 date string, kept verbatim.
    IsoDate(String),
    /// A formula cell. `formula` excludes the leading `=`.
    Formula {
        formula: String,
        /// The cached result, or `None` if the writing application didn't store one.
        /// An error result (`#N/A`, `#DIV/0!`, …) or an unreadable one is `Some(Null)`.
        cached: Option<Box<CellValue>>,
    },
}

impl CellValue {
    /// Whether PHP would see this cell as `''`: an empty cell, or empty text
    /// (inline, shared or `t="d"`). The reader trims these from the end of a row,
    /// and `empty_cells => null` turns the ones inside a row into `null`.
    /// A formula is never blank, whatever its cached value.
    pub fn is_blank(&self, shared: &SharedStrings) -> bool {
        match self {
            CellValue::Empty => true,
            CellValue::String(s) | CellValue::IsoDate(s) => s.is_empty(),
            CellValue::SharedString(i) => shared.get(*i).is_some_and(str::is_empty),
            _ => false,
        }
    }

    /// Build a numeric value with OpenSpout's int-vs-float rule:
    /// integral values that fit in i64 become `Int`.
    pub fn number(v: f64) -> CellValue {
        // i64 covers [-2^63, 2^63). Check the range explicitly: `as i64` saturates, and
        // 2^63 would otherwise round-trip through i64::MAX as f64 and pass.
        const LIMIT: f64 = 9_223_372_036_854_775_808.0; // 2^63
        if v.fract() == 0.0 && (-LIMIT..LIMIT).contains(&v) {
            CellValue::Int(v as i64)
        } else {
            CellValue::Float(v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_cells_are_empty_or_empty_text() {
        let shared = SharedStrings::from_strs(&["a", ""]);
        for blank in [
            CellValue::Empty,
            CellValue::String(String::new()),
            CellValue::IsoDate(String::new()),
            CellValue::SharedString(1),
        ] {
            assert!(blank.is_blank(&shared), "{blank:?}");
        }
        for not_blank in [
            CellValue::String(" ".into()),
            CellValue::SharedString(0),
            CellValue::SharedString(9), // out of range: not known to be empty
            CellValue::Int(0),
            CellValue::Null,
            CellValue::Bytes(Vec::new()),
            CellValue::Formula {
                formula: "\"\"".into(),
                cached: Some(Box::new(CellValue::String(String::new()))),
            },
        ] {
            assert!(!not_blank.is_blank(&shared), "{not_blank:?}");
        }
    }

    #[test]
    fn integral_numbers_become_int() {
        assert_eq!(CellValue::number(1500.0), CellValue::Int(1500));
        assert_eq!(CellValue::number(-3.0), CellValue::Int(-3));
        assert_eq!(CellValue::number(0.075), CellValue::Float(0.075));
    }

    #[test]
    fn out_of_range_and_non_finite_stay_float() {
        assert_eq!(CellValue::number(1e20), CellValue::Float(1e20));
        let two_pow_63 = 9_223_372_036_854_775_808.0;
        assert_eq!(CellValue::number(two_pow_63), CellValue::Float(two_pow_63));
        assert_eq!(CellValue::number(-two_pow_63), CellValue::Int(i64::MIN));
        assert!(matches!(CellValue::number(f64::NAN), CellValue::Float(_)));
        assert_eq!(
            CellValue::number(f64::INFINITY),
            CellValue::Float(f64::INFINITY)
        );
    }
}
