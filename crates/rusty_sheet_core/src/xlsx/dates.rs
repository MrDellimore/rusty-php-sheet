//! Excel serial dates, following OpenSpout's arithmetic exactly:
//! `base 1899-12-30 + trunc(serial) days + round(fmod(serial, 1) × 86400) seconds`.
//!
//! OpenSpout doesn't model Excel's fictitious 1900-02-29, so serial 1 is 1899-12-31
//! (Excel displays 1900-01-01). We match it; see `docs/parity-notes.md`.

/// Days between the 1900 and 1904 systems: 1904-01-01 is serial 1462 in the 1900 system.
pub const SYSTEM_1904_OFFSET: f64 = 1462.0;

/// OpenSpout's `isValidTimestampValue`, applied to the serial *as stored* in the file.
pub fn is_valid_serial(serial: f64, date1904: bool) -> bool {
    if date1904 {
        (-695_055.0..=2_957_003.999_988_4).contains(&serial)
    } else {
        (-693_593.0..=2_958_465.999_988_4).contains(&serial)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTimeParts {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// 1900-system serial → calendar date and time, with OpenSpout's rounding.
pub fn serial_to_parts(serial: f64) -> DateTimeParts {
    const BASE_DAYS: i64 = -25_569; // 1899-12-30 relative to 1970-01-01
    let days = serial.trunc() as i64;
    // `%` on f64 is fmod (sign follows the dividend); round() is half away from zero, like PHP.
    let seconds = ((serial % 1.0) * 86_400.0).round() as i64;
    let total = (BASE_DAYS + days) * 86_400 + seconds;
    let (day_number, secs_of_day) = (total.div_euclid(86_400), total.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(day_number);
    DateTimeParts {
        year,
        month,
        day,
        hour: (secs_of_day / 3600) as u32,
        minute: (secs_of_day % 3600 / 60) as u32,
        second: (secs_of_day % 60) as u32,
    }
}

/// A serial as sheet-stream's `CellNormalizer` (serial mode) yields it: rebuilt from
/// OpenSpout's second-rounded datetime, `int` when there's no time part.
///
/// Deliberate difference: sheet-stream subtracts one more day for dates before
/// 1900-03-01, but OpenSpout never added it, so every time-only value comes out a
/// day early (18:00 → -0.25). We return the true serial (18:00 → 0.75).
pub fn normalised_serial(serial: f64) -> SerialValue {
    let days = serial.trunc() as i64;
    let seconds = ((serial % 1.0) * 86_400.0).round() as i64;
    let total = days * 86_400 + seconds;
    let (d, s) = (total.div_euclid(86_400), total.rem_euclid(86_400));
    if s == 0 {
        SerialValue::Int(d)
    } else {
        SerialValue::Float(d as f64 + s as f64 / 86_400.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SerialValue {
    Int(i64),
    Float(f64),
}

/// OpenSpout's `createDateIntervalFromHours`: a day fraction split into whole
/// hours (may exceed 24), minutes and rounded seconds, with carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurationParts {
    pub negative: bool,
    pub hours: i64,
    pub minutes: i64,
    pub seconds: i64,
}

pub fn duration_parts(day_fraction: f64) -> DurationParts {
    let mut time = day_fraction.abs() * 24.0;
    let mut hours = time.floor();
    time = (time - hours) * 60.0;
    let mut minutes = time.floor() as i64;
    time = (time - minutes as f64) * 60.0;
    let mut seconds = time.round() as i64;
    if seconds == 60 {
        seconds = 0;
        minutes += 1;
    }
    if minutes == 60 {
        minutes = 0;
        hours += 1.0;
    }
    DurationParts {
        negative: day_fraction < 0.0,
        hours: hours as i64,
        minutes,
        seconds,
    }
}

impl DurationParts {
    /// sheet-stream's `intervalToSerial`: back to a day fraction, `int` if whole.
    pub fn to_serial(self) -> SerialValue {
        let secs = self.hours * 3600 + self.minutes * 60 + self.seconds;
        let mut serial = secs as f64 / 86_400.0;
        if self.negative {
            serial = -serial;
        }
        if serial == serial.floor() {
            SerialValue::Int(serial as i64)
        } else {
            SerialValue::Float(serial)
        }
    }

    /// ISO 8601 spec for PHP's `new DateInterval(...)`, as OpenSpout builds it.
    pub fn interval_spec(self) -> String {
        format!("P0DT{}H{}M{}S", self.hours, self.minutes, self.seconds)
    }
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian.
/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(serial: f64) -> String {
        let p = serial_to_parts(serial);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            p.year, p.month, p.day, p.hour, p.minute, p.second
        )
    }

    /// Expected values are OpenSpout's output for tests/fixtures/edge/dates.xlsx.
    #[test]
    fn matches_openspout() {
        assert_eq!(fmt(45823.0), "2025-06-15 00:00:00");
        assert_eq!(fmt(45823.5), "2025-06-15 12:00:00");
        assert_eq!(fmt(0.75), "1899-12-30 18:00:00");
        assert_eq!(fmt(1.0), "1899-12-31 00:00:00");
        assert_eq!(fmt(59.0), "1900-02-27 00:00:00");
        assert_eq!(fmt(60.0), "1900-02-28 00:00:00");
        assert_eq!(fmt(61.0), "1900-03-01 00:00:00");
        assert_eq!(fmt(0.0), "1899-12-30 00:00:00");
        assert_eq!(fmt(45823.999988426), "2025-06-15 23:59:59");
    }

    #[test]
    fn system_1904_shift() {
        assert_eq!(fmt(SYSTEM_1904_OFFSET), "1904-01-01 00:00:00");
        assert_eq!(fmt(44361.25 + SYSTEM_1904_OFFSET), "2025-06-15 06:00:00");
    }

    #[test]
    fn negative_serials() {
        assert_eq!(fmt(-1.0), "1899-12-29 00:00:00");
        assert_eq!(fmt(-0.5), "1899-12-29 12:00:00");
    }

    #[test]
    fn rounding_to_next_day() {
        // 0.9999999 days rounds to 86400 seconds, i.e. midnight of the next day.
        assert_eq!(fmt(10.9999999), "1900-01-10 00:00:00");
    }

    #[test]
    fn normalised_serials() {
        use SerialValue::*;
        assert_eq!(normalised_serial(45823.0), Int(45823));
        assert_eq!(normalised_serial(45823.5), Float(45823.5));
        assert_eq!(normalised_serial(0.75), Float(0.75)); // sheet-stream: -0.25 (bug)
        assert_eq!(normalised_serial(1.0), Int(1)); // sheet-stream: 0 (bug)
        assert_eq!(normalised_serial(-0.5), Float(-0.5));
        // Rounded to whole seconds, like the DateTime round trip.
        assert_eq!(
            normalised_serial(45823.999988426),
            Float(45823.0 + 86_399.0 / 86_400.0)
        );
        assert_eq!(normalised_serial(10.9999999), Int(11));
    }

    #[test]
    fn durations() {
        let p = duration_parts(1.5);
        assert_eq!(
            (p.hours, p.minutes, p.seconds, p.negative),
            (36, 0, 0, false)
        );
        assert_eq!(p.interval_spec(), "P0DT36H0M0S");
        assert_eq!(p.to_serial(), SerialValue::Float(1.5));
        assert_eq!(duration_parts(1.0).to_serial(), SerialValue::Int(1));
        let n = duration_parts(-0.25);
        assert!(n.negative);
        assert_eq!(n.to_serial(), SerialValue::Float(-0.25));
        // 59.6 seconds rounds up and carries into minutes and hours.
        let c = duration_parts((59.0 * 60.0 + 59.6) / 86_400.0);
        assert_eq!((c.hours, c.minutes, c.seconds), (1, 0, 0));
    }

    #[test]
    fn validity_range() {
        assert!(is_valid_serial(0.0, false));
        assert!(is_valid_serial(2_958_465.5, false));
        assert!(!is_valid_serial(2_958_466.0, false));
        assert!(!is_valid_serial(-693_594.0, false));
        assert!(is_valid_serial(-695_055.0, true));
        assert!(!is_valid_serial(2_957_004.0, true));
    }
}
