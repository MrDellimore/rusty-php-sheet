//! Render dates and durations the way OpenSpout's `SHOULD_FORMAT_DATES` does:
//! an Excel format code is converted to a PHP `date()` format
//! (`DateFormatHelper::toPHPDateFormat`, OpenSpout 5.15), which PHP then applies.
//!
//! [`format`] implements PHP's `date()` for the characters that conversion normally
//! produces. For anything else (e.g. timezone characters, which depend on PHP's
//! default timezone) it returns `None`, and the caller lets PHP format the value.

use super::dates::{DateTimeParts, DurationParts};

/// Port of OpenSpout 5.15's `DateFormatHelper::toPHPDateFormat`, quirks included
/// (e.g. only a *leading* `[$…]` is stripped, so `[Red]` survives and its letters
/// become format characters, exactly as in OpenSpout).
pub fn to_php_date_format(excel: &str) -> String {
    // 1. `^(?:\[\$[^\]]+?\])?([^;]*).*` → drop a leading locale block and any `;` sections.
    let mut s = excel;
    if let Some(rest) = s.strip_prefix("[$") {
        if let Some(close) = rest.find(']') {
            if close > 0 {
                s = &rest[close + 1..];
            }
        }
    }
    let s = s.split(';').next().unwrap_or("");

    // 2. `"[^"]*"` kept; `\X` (letter) → `"X"`; `\X` (other) → `X`.
    let mut escaped = String::with_capacity(s.len() + 8);
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => {
                if let Some(close) = s[i + 1..].find('"') {
                    escaped.push_str(&s[i..i + 1 + close + 1]);
                    while chars.peek().is_some_and(|&(j, _)| j <= i + 1 + close) {
                        chars.next();
                    }
                } else {
                    escaped.push('"');
                }
            }
            '\\' => match chars.peek().copied() {
                Some((_, n)) if n.is_ascii_alphabetic() => {
                    chars.next();
                    escaped.push('"');
                    escaped.push(n);
                    escaped.push('"');
                }
                Some((_, n)) if n != '\n' => {
                    chars.next();
                    escaped.push(n);
                }
                _ => escaped.push('\\'),
            },
            _ => escaped.push(c),
        }
    }

    // 3. Split on quotes: even parts are format, odd parts are literal text.
    let parts: Vec<&str> = escaped.split('"').collect();
    let format_text: String = parts.iter().step_by(2).copied().collect();
    let twelve_hour = format_text.to_ascii_lowercase().contains("am/pm");

    let mut out = String::with_capacity(escaped.len() + 8);
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push('"');
        }
        if i % 2 == 1 {
            out.push_str(part);
            continue;
        }
        let general = strtr(&part.to_ascii_lowercase(), GENERAL);
        out.push_str(&strtr(
            &general,
            if twelve_hour { HOUR_12 } else { HOUR_24 },
        ));
    }

    // 4. `"(.+?)"` → every character backslash-escaped.
    escape_quoted(&out)
}

/// Longest keys first, as PHP's `strtr` tries them.
const GENERAL: &[(&str, &str)] = &[
    ("mmmmm", "M"),
    ("am/pm", "A"),
    ("yyyy", "Y"),
    ("mmmm", "F"),
    ("dddd", "l"),
    (":mm", ":i"),
    ("mm:", "i:"),
    ("mmm", "M"),
    ("ddd", "D"),
    ("ss", "s"),
    (".s", ""),
    ("yy", "y"),
    ("mm", "m"),
    ("dd", "d"),
    ("e", "Y"),
    ("m", "n"),
    ("d", "j"),
];
const HOUR_12: &[(&str, &str)] = &[("hh", "h"), ("h", "g")];
const HOUR_24: &[(&str, &str)] = &[("hh", "H"), ("h", "G")];

/// PHP `strtr($s, $map)`: at each position the longest matching key wins; replaced
/// text is never rescanned.
fn strtr(s: &str, map: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    'outer: while i < s.len() {
        for (from, to) in map {
            if s[i..].starts_with(from) {
                out.push_str(to);
                i += from.len();
                continue 'outer;
            }
        }
        let c = s[i..].chars().next().expect("in bounds");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// `preg_replace_callback('/"(.+?)"/', …)`: quoted text becomes `\x\y\z`.
fn escape_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'"' {
            // `.+?` needs at least one character (not a newline) before the closing quote.
            let close = s[i + 1..]
                .char_indices()
                .skip(1)
                .find(|&(_, c)| c == '"')
                .map(|(j, _)| i + 1 + j)
                .filter(|&j| !s[i + 1..j].contains('\n'));
            if let Some(j) = close {
                for c in s[i + 1..j].chars() {
                    out.push('\\');
                    out.push(c);
                }
                i = j + 1;
                continue;
            }
        }
        let c = s[i..].chars().next().expect("in bounds");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// PHP `DateTimeInterface::format()` for a wall-clock date. `None` if the format
/// uses a character this doesn't implement; let PHP format it then.
pub fn format(p: &DateTimeParts, php_format: &str) -> Option<String> {
    let leap = is_leap(p.year);
    let day_of_year = day_of_year(p.year, p.month, p.day, leap);
    let weekday = weekday(p.year, p.month, p.day); // 0 = Sunday
    let hour12 = match p.hour % 12 {
        0 => 12,
        h => h,
    };

    let mut out = String::with_capacity(php_format.len() * 3);
    let mut chars = php_format.chars();
    while let Some(c) = chars.next() {
        use std::fmt::Write;
        let _ = match c {
            'd' => write!(out, "{:02}", p.day),
            'D' => write!(out, "{}", &DAYS[weekday][..3]),
            'j' => write!(out, "{}", p.day),
            'l' => write!(out, "{}", DAYS[weekday]),
            'N' => write!(out, "{}", if weekday == 0 { 7 } else { weekday }),
            'S' => write!(out, "{}", ordinal_suffix(p.day)),
            'w' => write!(out, "{weekday}"),
            'z' => write!(out, "{day_of_year}"),
            'F' => write!(out, "{}", MONTHS[p.month as usize - 1]),
            'M' => write!(out, "{}", &MONTHS[p.month as usize - 1][..3]),
            'm' => write!(out, "{:02}", p.month),
            'n' => write!(out, "{}", p.month),
            't' => write!(out, "{}", days_in_month(p.month, leap)),
            'L' => write!(out, "{}", u8::from(leap)),
            'Y' => write!(out, "{}", php_year(p.year)),
            'y' => write!(out, "{:02}", p.year.rem_euclid(100)),
            'a' => write!(out, "{}", if p.hour < 12 { "am" } else { "pm" }),
            'A' => write!(out, "{}", if p.hour < 12 { "AM" } else { "PM" }),
            'g' => write!(out, "{hour12}"),
            'h' => write!(out, "{hour12:02}"),
            'G' => write!(out, "{}", p.hour),
            'H' => write!(out, "{:02}", p.hour),
            'i' => write!(out, "{:02}", p.minute),
            's' => write!(out, "{:02}", p.second),
            'u' => write!(out, "000000"),
            'v' => write!(out, "000"),
            // PHP's date_format() skips past the backslash and copies the next byte;
            // at the end of the format that byte is the string's terminating NUL.
            '\\' => {
                out.push(chars.next().unwrap_or('\0'));
                Ok(())
            }
            c if c.is_ascii_alphabetic() => return None, // other PHP format character
            c => write!(out, "{c}"),
        };
    }
    Some(out)
}

/// PHP's `Y`: at least four digits, `-` for years before year 0.
fn php_year(y: i64) -> String {
    if y < 0 {
        format!("-{:04}", -y)
    } else {
        format!("{y:04}")
    }
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(m: u32, leap: bool) -> u32 {
    match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn day_of_year(y: i64, m: u32, d: u32, leap: bool) -> u32 {
    let _ = y;
    (1..m).map(|mm| days_in_month(mm, leap)).sum::<u32>() + d - 1
}

/// 0 = Sunday … 6 = Saturday (Sakamoto's method).
fn weekday(y: i64, m: u32, d: u32) -> usize {
    const T: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    (y + y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400) + T[m as usize - 1] + i64::from(d))
        .rem_euclid(7) as usize
}

fn ordinal_suffix(day: u32) -> &'static str {
    match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    }
}

/// OpenSpout's `DateIntervalFormatHelper::formatDateInterval` for a duration code
/// such as `[h]:mm:ss` (already validated by `numfmt::is_duration_format`).
pub fn format_duration(excel: &str, d: &DurationParts) -> String {
    let units: Vec<String> = excel
        .replace(['[', ']'], "")
        .split(':')
        .map(str::to_string)
        .collect();
    let (mut h, mut m, mut s) = (d.hours, d.minutes, d.seconds);
    match units.first().and_then(|u| u.chars().next()) {
        Some('m') => {
            m += h * 60;
            h = 0;
        }
        Some('s') => {
            s += m * 60 + h * 3600;
            m = 0;
            h = 0;
        }
        _ => {}
    }
    let mut out = String::new();
    if d.negative {
        out.push('-');
    }
    let rendered: Vec<String> = units
        .iter()
        .map(|u| match u.as_str() {
            "hh" => format!("{h:02}"),
            "h" => h.to_string(),
            "mm" => format!("{m:02}"),
            "m" => m.to_string(),
            "ss" => format!("{s:02}"),
            "s" => s.to_string(),
            _ => String::new(),
        })
        .collect();
    out.push_str(&rendered.join(":"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xlsx::dates::{duration_parts, serial_to_parts};

    /// Expected values produced by calling OpenSpout 5.15.3 DateFormatHelper::toPHPDateFormat.
    #[test]
    fn converts_like_openspout() {
        for (excel, php) in [
            ("m/d/yyyy", "n/j/Y"),
            ("d-mmm-yy", "j-M-y"),
            ("h:mm AM/PM", "g:i A"),
            ("h:mm:ss AM/PM", "g:i:s A"),
            ("m/d/yyyy h:mm", "n/j/Y G:i"),
            ("mm:ss", "i:s"),
            ("mm:ss.0", "i:s.0"),
            ("yyyy-mm-dd hh:mm:ss", "Y-m-d H:i:s"),
            ("[$-409]mmmm d, yyyy;@", "F j, Y"),
            ("\"Due \"yyyy", "\\D\\u\\e\\ Y"),
            ("yyyy-mm-dd\\Thh", "Y-m-d\\TH"),
            ("dddd, mmmm d", "l, F j"),
            ("[Red]yyyy", "[rYj]Y"),
            ("hh:mm \"AM/PM\"", "H:i \\A\\M\\/\\P\\M"),
            ("mmmmm", "M"),
            ("e", "Y"),
        ] {
            assert_eq!(to_php_date_format(excel), php, "{excel}");
        }
    }

    #[test]
    fn renders_like_php_date() {
        let p = serial_to_parts(45823.5); // Sunday 2025-06-15 12:00
        assert_eq!(
            format(&p, "Y-m-d H:i:s").as_deref(),
            Some("2025-06-15 12:00:00")
        );
        assert_eq!(
            format(&p, "l, F jS, y").as_deref(),
            Some("Sunday, June 15th, 25")
        );
        assert_eq!(
            format(&p, "D M n/j g:i A").as_deref(),
            Some("Sun Jun 6/15 12:00 PM")
        );
        assert_eq!(format(&p, "N w z t L").as_deref(), Some("7 0 165 30 0"));
        assert_eq!(format(&p, "\\D\\a\\y d").as_deref(), Some("Day 15"));
        let midnight = serial_to_parts(45823.0);
        assert_eq!(format(&midnight, "g h a").as_deref(), Some("12 12 am"));
        let early = serial_to_parts(1.0); // 1899-12-31, a Sunday
        assert_eq!(
            format(&early, "Y-m-d l").as_deref(),
            Some("1899-12-31 Sunday")
        );
        // Timezone-dependent characters are left to PHP.
        assert_eq!(format(&p, "Y T"), None);
        assert_eq!(format(&p, "c"), None);
    }

    #[test]
    fn suffixes() {
        let s: Vec<&str> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 31]
            .iter()
            .map(|&d| ordinal_suffix(d))
            .collect();
        assert_eq!(
            s,
            [
                "st", "nd", "rd", "th", "th", "th", "th", "st", "nd", "rd", "st"
            ]
        );
    }

    #[test]
    fn durations_like_openspout() {
        let d = duration_parts(1.5 + 30.0 / 1440.0); // 36:30:00
        assert_eq!(format_duration("[h]:mm:ss", &d), "36:30:00");
        assert_eq!(format_duration("[hh]:mm", &d), "36:30");
        assert_eq!(format_duration("[mm]:ss", &d), "2190:00");
        assert_eq!(format_duration("[s]", &d), "131400");
        let n = duration_parts(-0.25);
        assert_eq!(format_duration("[h]:mm", &n), "-6:00");
        let small = duration_parts(5.0 / 86_400.0);
        assert_eq!(format_duration("[h]:mm:ss", &small), "0:00:05");
    }

    /// Expected values from OpenSpout 5.15.3's toPHPDateFormat and PHP 8.5's
    /// DateTimeInterface::format.
    #[test]
    fn rare_format_codes_like_openspout_and_php() {
        for (excel, php) in [
            ("[$]yyyy", "[$]Y"),            // `[$]` is not a locale block
            ("[$yyyy", "[$Y"),              // nor is an unclosed `[$`
            ("\"open yyyy", "\"open yyyy"), // unterminated quote
            ("\"\" yyyy", "\"\" Y"),
            ("\"a\nb\" yyyy", "\"a\nb\" Y"), // a quote can't span a line break
            ("yyyy\\\\", "Y\\"),             // escaped backslash
            ("yyyy\\\n", "Y\\\n"),           // a backslash before a line break stays
        ] {
            assert_eq!(to_php_date_format(excel), php, "{excel:?}");
        }
        let p = serial_to_parts(43_894.212_581_018_52); // 2020-03-04 05:06:07
        assert_eq!(format(&p, "u v").as_deref(), Some("000000 000"));
        assert_eq!(format(&p, "Y \\\\").as_deref(), Some("2020 \\"));
        // Characters not rendered here make the caller fall back to PHP.
        assert_eq!(format(&p, "Y x"), None);
        // PHP appends the terminating NUL when a format ends in a lone backslash.
        assert_eq!(format(&p, "Y\\").as_deref(), Some("2020\0"));
        assert_eq!(php_year(-44), "-0044");
    }
}
