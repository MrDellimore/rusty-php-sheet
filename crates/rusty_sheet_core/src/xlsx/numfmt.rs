//! Number-format classification, ported from OpenSpout 5.13's `StyleManager` and
//! `DateIntervalFormatHelper` so date detection matches it exactly, quirks included.

/// Built-in date/time formats as OpenSpout maps them (not always the ECMA codes).
pub fn builtin_date_format(id: u32) -> Option<&'static str> {
    Some(match id {
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        _ => return None,
    })
}

/// OpenSpout's `isFormatCodeCustomDateFormat`.
pub fn is_date_format_code(code: &str) -> bool {
    if code.is_empty() || code.eq_ignore_ascii_case("General") {
        return false;
    }
    let stripped = strip_exponents(&strip_quoted(&strip_brackets(code)));
    has_date_token(stripped.as_bytes())
}

/// OpenSpout's `isDurationFormat`: the whole code must match
/// `^(\[hh?](:mm(:ss)?)?|\[mm?](:ss)?|\[ss?])$` (case-sensitive).
pub fn is_duration_format(code: &str) -> bool {
    let rest = |unit: &str| code.strip_prefix(&format!("[{unit}]"));
    for unit in ["hh", "h"] {
        if let Some(r) = rest(unit) {
            if matches!(r, "" | ":mm" | ":mm:ss") {
                return true;
            }
        }
    }
    for unit in ["mm", "m"] {
        if let Some(r) = rest(unit) {
            if matches!(r, "" | ":ss") {
                return true;
            }
        }
    }
    matches!(code, "[s]" | "[ss]")
}

fn preceded_by_backslash(s: &[u8], i: usize) -> bool {
    i > 0 && s[i - 1] == b'\\'
}

/// Remove `(?<!\\)\[.+?(?<!\\)\]`: bracketed sections like `[Red]` or `[$-409]`.
fn strip_brackets(code: &str) -> String {
    let s = code.as_bytes();
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'[' && !preceded_by_backslash(s, i) {
            // `.+?` needs at least one character before the closing bracket.
            let close = (i + 2..s.len()).find(|&k| s[k] == b']' && s[k - 1] != b'\\');
            if let Some(k) = close {
                i = k + 1;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Remove `"[^"]+"`: non-empty quoted literals.
fn strip_quoted(code: &str) -> String {
    let s = code.as_bytes();
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'"' {
            if let Some(off) = s[i + 1..].iter().position(|&b| b == b'"') {
                if off > 0 {
                    i += off + 2;
                    continue;
                }
            }
        }
        out.push(s[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Remove `(?<!\\)[eE][+-]?[0#?]+`: scientific-notation exponents.
fn strip_exponents(code: &str) -> String {
    let s = code.as_bytes();
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if matches!(s[i], b'e' | b'E') && !preceded_by_backslash(s, i) {
            let mut j = i + 1;
            if j < s.len() && matches!(s[j], b'+' | b'-') {
                j += 1;
            }
            let digits = s[j..]
                .iter()
                .take_while(|&&b| matches!(b, b'0' | b'#' | b'?'))
                .count();
            if digits > 0 {
                i = j + digits;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Any of `e`, `yy`, `m`, `d`, `h`, `s` (case-insensitive) not preceded by `\`.
fn has_date_token(s: &[u8]) -> bool {
    (0..s.len()).any(|i| {
        if preceded_by_backslash(s, i) {
            return false;
        }
        match s[i].to_ascii_lowercase() {
            b'e' | b'm' | b'd' | b'h' | b's' => true,
            b'y' => s.get(i + 1).is_some_and(|b| b.eq_ignore_ascii_case(&b'y')),
            _ => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_detected() {
        for code in [
            "yyyy-mm-dd",
            "dd/mm/yyyy hh:mm",
            "[$-409]mmmm d, yyyy;@",
            "\"Due \"yyyy",
            "mm:ss",
            "d-mmm",
            "h:mm AM/PM",
            "[h]:mm:ss",
            "yy",
        ] {
            assert!(is_date_format_code(code), "{code} should be a date");
        }
    }

    #[test]
    fn non_dates_rejected() {
        for code in [
            "",
            "General",
            "general",
            "0.00",
            "#,##0",
            "\"y\"0",
            "[Red]0.00",
            "0.00E+00",
            "0,00e-0#",
            "\\d\\a\\y 0",
            "y0",
        ] {
            assert!(!is_date_format_code(code), "{code} should not be a date");
        }
    }

    #[test]
    fn openspout_quirks_are_preserved() {
        // The General check happens before brackets are stripped, so the 'e' counts.
        assert!(is_date_format_code("[Red]General"));
        // An empty quoted string isn't removed, so its surroundings are still scanned.
        assert!(!is_date_format_code("\"\"0"));
    }

    #[test]
    fn durations() {
        for code in [
            "[h]:mm:ss",
            "[hh]:mm",
            "[h]",
            "[mm]:ss",
            "[m]",
            "[ss]",
            "[s]",
        ] {
            assert!(is_duration_format(code), "{code}");
        }
        for code in ["h:mm", "[h]:mm:ss.0", "[hhh]", "[H]:mm", "mm:ss", "[h]:ss"] {
            assert!(!is_duration_format(code), "{code}");
        }
    }

    #[test]
    fn builtins() {
        assert_eq!(builtin_date_format(14), Some("m/d/yyyy"));
        assert_eq!(builtin_date_format(46), Some("[h]:mm:ss"));
        assert_eq!(builtin_date_format(4), None);
    }
}
