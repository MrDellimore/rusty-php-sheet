//! Writers that produce the same files as OpenSpout's XLSX and CSV writers.

pub mod autosize;
pub mod csv;
mod escape;
pub mod style;
pub mod xlsx;

pub use csv::{CsvWriter, CsvWriterOptions};
pub use style::{Border, BorderLine, BorderPart, BorderSide, BorderWidth, Style};
pub use xlsx::{Cell, CellData, CellRange, Link, MAX_COLUMNS, MAX_ROWS, XlsxWriter};

#[cfg(test)]
pub(crate) use escape::escape as escape_for_tests;

/// A date/time as an Excel serial, as OpenSpout's `DateHelper::toExcel` computes it
/// from the wall-clock fields (PhpSpreadsheet's Julian-day method, with Excel's
/// fictitious 1900-02-29 accounted for).
pub fn excel_serial(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> f64 {
    let leap_fudge = if year == 1900 && month <= 2 { 0 } else { 1 };
    let (month, year) = if month > 2 {
        (month - 3, year)
    } else {
        (month + 9, year - 1)
    };
    // PHP: (int) substr((string) $year, 0, 2) and (int) substr((string) $year, 2, 2).
    let ys = year.to_string();
    let century: i64 = ys
        .get(0..2.min(ys.len()))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let decade: i64 = ys
        .get(2..4.min(ys.len()))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let date = (146_097 * century).div_euclid(4)
        + (1461 * decade).div_euclid(4)
        + (153 * month + 2).div_euclid(5)
        + day
        + 1_721_119
        - 2_415_020
        + leap_fudge;
    let time = (hour * 3600 + minute * 60 + second) as f64 / 86_400.0;
    date as f64 + time
}

/// OpenSpout's `DateIntervalHelper::toExcel` (years and months use average lengths).
#[allow(clippy::too_many_arguments)]
pub fn interval_serial(y: i64, m: i64, d: i64, h: i64, i: i64, s: i64, invert: bool) -> f64 {
    let days = y as f64 * 365.25
        + m as f64 * 30.437
        + d as f64
        + h as f64 / 24.0
        + i as f64 / 24.0 / 60.0
        + s as f64 / 24.0 / 60.0 / 60.0;
    if invert { -days } else { days }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values from OpenSpout's DateHelper::toExcel.
    #[test]
    fn serials_like_openspout() {
        assert_eq!(excel_serial(2025, 6, 15, 0, 0, 0), 45823.0);
        assert!((excel_serial(2025, 6, 15, 14, 30, 0) - 45823.604166666664).abs() < 1e-9);
        assert_eq!(excel_serial(1900, 1, 1, 0, 0, 0), 1.0);
        assert_eq!(excel_serial(1900, 2, 28, 0, 0, 0), 59.0);
        assert_eq!(excel_serial(1900, 3, 1, 0, 0, 0), 61.0);
        assert_eq!(
            excel_serial(1999, 12, 31, 23, 59, 59),
            36525.0 + 86_399.0 / 86_400.0
        );
    }
}
