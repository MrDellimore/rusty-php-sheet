//! Column widths for auto-sized columns: a port of PhpSpreadsheet 5.11's
//! approximate mode (`Shared\Font::calculateColumnWidth` with `AUTOSIZE_METHOD_APPROX`,
//! as `Worksheet::calculateColumnWidths` drives it), which is what litportal gets
//! today. Each cell's value is rendered through its number format, its characters
//! counted (`mb_strwidth`: East Asian wide characters count twice), and the count
//! turned into pixels by a per-font factor and the font size. The widest cell wins.
//!
//! Approximations, as in PhpSpreadsheet: bold and italic don't change the width;
//! only Arial, Verdana and (for every other font) Calibri factors exist.

use crate::xlsx::{numfmt, php_date};

/// `Font::DEFAULT_COLUMN_WIDTHS`: (font, size) → (px, width) of a default column.
const DEFAULT_COLUMN_WIDTHS: &[(&str, u32, u32, f64)] = &[
    ("Arial", 1, 24, 12.0),
    ("Arial", 2, 24, 12.0),
    ("Arial", 3, 32, 10.664_062_5),
    ("Arial", 4, 32, 10.664_062_5),
    ("Arial", 5, 40, 10.0),
    ("Arial", 6, 48, 9.597_656_25),
    ("Arial", 7, 48, 9.597_656_25),
    ("Arial", 8, 56, 9.332_031_25),
    ("Arial", 9, 64, 9.140_625),
    ("Arial", 10, 64, 9.140_625),
    ("Calibri", 1, 24, 12.0),
    ("Calibri", 2, 24, 12.0),
    ("Calibri", 3, 32, 10.664_062_5),
    ("Calibri", 4, 32, 10.664_062_5),
    ("Calibri", 5, 40, 10.0),
    ("Calibri", 6, 48, 9.597_656_25),
    ("Calibri", 7, 48, 9.597_656_25),
    ("Calibri", 8, 56, 9.332_031_25),
    ("Calibri", 9, 56, 9.332_031_25),
    ("Calibri", 10, 64, 9.140_625),
    ("Calibri", 11, 64, 9.140_625),
    ("Verdana", 1, 24, 12.0),
    ("Verdana", 2, 24, 12.0),
    ("Verdana", 3, 32, 10.664_062_5),
    ("Verdana", 4, 32, 10.664_062_5),
    ("Verdana", 5, 40, 10.0),
    ("Verdana", 6, 48, 9.597_656_25),
    ("Verdana", 7, 48, 9.597_656_25),
    ("Verdana", 8, 64, 9.140_625),
    ("Verdana", 9, 72, 9.0),
    ("Verdana", 10, 72, 9.0),
];

/// A font as far as width estimation cares.
#[derive(Debug, Clone, Copy)]
pub struct Font<'a> {
    pub name: &'a str,
    pub size: f64,
}

/// What `calculateColumnWidth` needs to know about one cell.
#[derive(Debug, Clone, Copy)]
pub struct Measure<'a> {
    pub font: Font<'a>,
    /// The workbook's default font (the Normal style).
    pub default_font: Font<'a>,
    pub rotation: i64,
    /// The cell is in the header row of an autofilter: room for the dropdown.
    pub filter: bool,
    /// Horizontally centred text counts as one level of indent.
    pub indent: i64,
}

/// `Font::calculateColumnWidth`, rounded to 3 decimals as `calculateColumnWidths` does.
pub fn column_width(text: &str, m: &Measure<'_>) -> f64 {
    if text.contains('\n') {
        // Each line on its own, without rotation and indent (as PhpSpreadsheet does).
        let line = Measure {
            rotation: 0,
            indent: 0,
            ..*m
        };
        return text
            .split('\n')
            .map(|l| column_width(l, &line))
            .fold(f64::MIN, f64::max);
    }
    let padding_chars = (if m.filter { 3 } else { 1 }) + (m.indent * 2).max(0) as usize;
    let adjust = text_width_px(padding_chars, m.font, 0);
    let px = text_width_px(mb_strwidth(text), m.font, m.rotation) + adjust;
    let width = pixels_to_cell_dimension(px, m.default_font);
    round_to(round_to(width, 4), 3)
}

/// `Font::getTextWidthPixelsApprox` for a text of `chars` display columns.
fn text_width_px(chars: usize, font: Font<'_>, rotation: i64) -> i64 {
    let chars = chars as f64;
    let mut px = match font.name {
        // PHP: (int) (8 * $chars) * $size / 10
        "Arial" | "Verdana" => (8.0 * chars).trunc() * font.size / 10.0,
        _ => (8.26 * chars).trunc() * font.size / 11.0,
    };
    if rotation != 0 {
        if rotation == 255 {
            px = 4.0; // stacked text
        } else {
            let r = (rotation as f64).to_radians();
            px = px * r.cos() + font.size * r.sin().abs() / 5.0;
        }
    }
    px as i64
}

/// `Drawing::pixelsToCellDimension`.
fn pixels_to_cell_dimension(px: i64, default: Font<'_>) -> f64 {
    let px = px as f64;
    if default.size.fract() == 0.0
        && let Some(&(_, _, table_px, width)) = DEFAULT_COLUMN_WIDTHS
            .iter()
            .find(|(n, s, ..)| *n == default.name && f64::from(*s) == default.size)
    {
        return px * width / f64::from(table_px);
    }
    px * 11.0 * 9.140_625 / 64.0 / default.size
}

/// PHP's `round($v, $places)`: half away from zero.
fn round_to(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}

/// `mb_strwidth()`: East Asian wide and fullwidth characters count 2.
pub fn mb_strwidth(s: &str) -> usize {
    s.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum()
}

fn is_wide(c: char) -> bool {
    matches!(u32::from(c),
        0x1100..=0x115F | 0x231A..=0x231B | 0x2329..=0x232A | 0x23E9..=0x23EC | 0x23F0
        | 0x23F3 | 0x25FD..=0x25FE | 0x2614..=0x2615 | 0x2648..=0x2653 | 0x267F | 0x2693
        | 0x26A1 | 0x26AA..=0x26AB | 0x26BD..=0x26BE | 0x26C4..=0x26C5 | 0x26CE | 0x26D4
        | 0x26EA | 0x26F2..=0x26F3 | 0x26F5 | 0x26FA | 0x26FD | 0x2705 | 0x270A..=0x270B
        | 0x2728 | 0x274C | 0x274E | 0x2753..=0x2755 | 0x2757 | 0x2795..=0x2797 | 0x27B0
        | 0x27BF | 0x2B1B..=0x2B1C | 0x2B50 | 0x2B55 | 0x2E80..=0x303E | 0x3041..=0x3247
        | 0x3250..=0x4DBF | 0x4E00..=0xA4CF | 0xA960..=0xA97F | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF | 0xFE10..=0xFE19 | 0xFE30..=0xFE6F | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 | 0x16FE0..=0x18CFF | 0x1B000..=0x1B2FF | 0x1F004 | 0x1F0CF
        | 0x1F18E | 0x1F191..=0x1F19A | 0x1F200..=0x1F251 | 0x1F300..=0x1F64F
        | 0x1F680..=0x1F6FF | 0x1F7E0..=0x1F7EB | 0x1F90C..=0x1F9FF | 0x1FA70..=0x1FAFF
        | 0x20000..=0x3FFFD)
}

/// The text PhpSpreadsheet measures for a number in `format` (`NumberFormat::
/// toFormattedString` of `convertToString($value)`): General, digit placeholders
/// with decimals, thousands separators and `?` padding, percentages, literal text,
/// `_x` spacing, `*x` fills, `[$€-409]` currency, colour and condition blocks, up to
/// three sections, `@`, fractions, digit masks such as `000-00-0000`, scientific
/// notation with a signed exponent, dates and elapsed times. Checked against
/// PhpSpreadsheet 5.11 in php/tests/Feature/AutoSizeTest.php. Known differences:
/// unsigned exponents (`0E0`), and dates PhpSpreadsheet renders by conventions of
/// its own (serial 0, negatives, 1900-01-01..02-28).
pub fn format_number(v: f64, format: Option<&str>) -> String {
    let Some(format) = format.filter(|f| !f.is_empty() && *f != "General" && *f != "@") else {
        return value_string(v);
    };
    FORMATS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(format) {
            cache.insert(format.to_string(), FormatKind::of(format));
        }
        cache[format].render(v, format)
    })
}

/// What a format code is, worked out once per code (a sheet has only a few).
enum FormatKind {
    /// A date or time: its PHP `date()` format.
    Date(String),
    Duration,
    Number,
}

thread_local! {
    static FORMATS: std::cell::RefCell<std::collections::HashMap<String, FormatKind>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

impl FormatKind {
    fn of(format: &str) -> Self {
        if !numfmt::is_date_format_code(format) {
            FormatKind::Number
        } else if numfmt::is_duration_format(format) {
            FormatKind::Duration
        } else {
            // Colour and condition blocks show nothing (OpenSpout's converter, made
            // for its formatted mode, would keep their letters); elapsed [h] stays.
            FormatKind::Date(php_date::to_php_date_format(&strip_blocks(format)))
        }
    }

    fn render(&self, v: f64, format: &str) -> String {
        match self {
            // A number that isn't a valid date is shown as the number (as PhpSpreadsheet does).
            FormatKind::Date(_) | FormatKind::Duration
                if !crate::xlsx::dates::is_valid_serial(v, false) =>
            {
                value_string(v)
            }
            // Elapsed time ([h]:mm …): total hours, minutes or seconds.
            FormatKind::Duration => {
                php_date::format_duration(format, &crate::xlsx::dates::duration_parts(v))
            }
            FormatKind::Date(php) => php_date::format(&crate::xlsx::dates::serial_to_parts(v), php)
                .unwrap_or_else(|| value_string(v)),
            FormatKind::Number => {
                let sections = split_sections(format);
                // Up to three sections: positive; negative (shown without its sign); zero.
                let (section, value, sign) = if v < 0.0 && sections.len() > 1 {
                    (sections[1], -v, "")
                } else if v == 0.0 && sections.len() > 2 {
                    (sections[2], v, "")
                } else if v < 0.0 {
                    (sections[0], -v, "-")
                } else {
                    (sections[0], v, "")
                };
                render_section(section, value, sign).unwrap_or_else(|| value_string(v))
            }
        }
    }
}

/// Drop `[…]` blocks other than elapsed-time units.
fn strip_blocks(format: &str) -> String {
    let mut out = String::with_capacity(format.len());
    let mut rest = format;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']') else {
            break;
        };
        out.push_str(&rest[..open]);
        let inner = &rest[open + 1..open + close];
        if !inner.is_empty()
            && inner
                .chars()
                .all(|c| matches!(c, 'h' | 'H' | 'm' | 'M' | 's' | 'S'))
        {
            out.push_str(&rest[open..=open + close]);
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

/// Split on `;` outside quotes and brackets.
fn split_sections(format: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut bracket, mut escaped) = (0, false, false, false);
    for (i, c) in format.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            '[' if !quoted => bracket = true,
            ']' if !quoted => bracket = false,
            ';' if !quoted && !bracket => {
                out.push(&format[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&format[start..]);
    out
}

/// One section with a non-negative value; `sign` goes right before the number,
/// after any literal prefix (`$-1.00`, as PhpSpreadsheet does). `None` for what
/// isn't covered.
fn render_section(section: &str, value: f64, sign: &str) -> Option<String> {
    if let Some(text) = render_scientific(section, value, sign) {
        return Some(text);
    }
    if let Some(text) = render_fraction(section, value, sign) {
        return Some(text);
    }
    if let Some(text) = render_mask(section, value, sign) {
        return Some(text);
    }
    // Literal text, placeholders and the % count, in order.
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut digits = String::new(); // the placeholder run: 0 # ? , .
    let mut percent = 0;
    let mut chars = section.chars().peekable();
    let mut seen_digits = false;
    let push_literal = |s: &str, seen: bool, prefix: &mut String, suffix: &mut String| {
        if seen {
            suffix.push_str(s);
        } else {
            prefix.push_str(s);
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut lit = String::new();
                for n in chars.by_ref() {
                    if n == '"' {
                        break;
                    }
                    lit.push(n);
                }
                push_literal(&lit, seen_digits, &mut prefix, &mut suffix);
            }
            '\\' => {
                if let Some(n) = chars.next() {
                    push_literal(&n.to_string(), seen_digits, &mut prefix, &mut suffix);
                }
            }
            '_' => {
                chars.next();
                push_literal(" ", seen_digits, &mut prefix, &mut suffix);
            }
            // `*x` repeats x to fill the cell; PhpSpreadsheet drops the `*`, keeps x.
            '*' => {}
            '[' => {
                let mut inner = String::new();
                for n in chars.by_ref() {
                    if n == ']' {
                        break;
                    }
                    inner.push(n);
                }
                // [$€-409] shows its symbol; colours and conditions show nothing.
                if let Some(cur) = inner.strip_prefix('$') {
                    let symbol = cur.split('-').next().unwrap_or("");
                    push_literal(symbol, seen_digits, &mut prefix, &mut suffix);
                }
            }
            '0' | '#' | '?' => {
                if !suffix.is_empty() {
                    return None; // placeholders split by literal text
                }
                seen_digits = true;
                digits.push(c);
            }
            ',' | '.' if seen_digits && suffix.is_empty() => digits.push(c),
            '%' => {
                percent += 1;
                push_literal("%", seen_digits, &mut prefix, &mut suffix);
            }
            // Text placeholder: the value as PhpSpreadsheet prints it.
            '@' => push_literal(&value_string(value), seen_digits, &mut prefix, &mut suffix),
            c => push_literal(&c.to_string(), seen_digits, &mut prefix, &mut suffix),
        }
    }
    if !seen_digits {
        return Some(format!("{prefix}{suffix}"));
    }
    let value = value * 100f64.powi(percent);
    let (int_part, dec_part) = digits.split_once('.').unwrap_or((&digits, ""));
    let thousands = int_part.contains(',');
    let min_int = int_part.chars().filter(|&c| c == '0').count();
    let max_dec = dec_part
        .chars()
        .filter(|c| matches!(c, '0' | '#' | '?'))
        .count();
    // PhpSpreadsheet pads `#` and `?` decimals like `0` (`#.##` shows .50).
    let dec_blanks = dec_part.contains('?');
    let rounded = format!("{value:.max_dec$}");
    let (int_digits, dec_digits) = match rounded.split_once('.') {
        Some((i, d)) => (i.to_string(), d.to_string()),
        None => (rounded.clone(), String::new()),
    };
    let mut int_digits = int_digits.trim_start_matches('0').to_string();
    while int_digits.len() < min_int {
        int_digits.insert(0, '0');
    }
    // `?` reserves a digit's width: blank where there is no digit.
    let int_spaces = int_part.chars().filter(|&c| c == '?').count();
    while int_digits.len() < min_int + int_spaces {
        int_digits.insert(0, ' ');
    }
    if thousands {
        let bytes: Vec<char> = int_digits.chars().collect();
        let mut grouped = String::new();
        for (i, ch) in bytes.iter().enumerate() {
            if i > 0 && (bytes.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(*ch);
        }
        int_digits = grouped;
    }
    let dec_digits = if dec_blanks {
        // PhpSpreadsheet's padValue: trailing zeros become spaces.
        let kept = dec_digits.trim_end_matches('0');
        format!("{kept}{}", " ".repeat(dec_digits.len() - kept.len()))
    } else {
        dec_digits
    };
    let number = if dec_digits.is_empty() {
        int_digits
    } else {
        format!("{int_digits}.{dec_digits}")
    };
    Some(format!("{prefix}{sign}{number}{suffix}"))
}

/// PhpSpreadsheet's `FractionFormatter` for `# ?/?`, `?/??`, `# ?/8` and the like.
fn render_fraction(section: &str, value: f64, sign: &str) -> Option<String> {
    let slash = section.find('/')?;
    let before = &section[..slash];
    let after = &section[slash + 1..];
    if !before.ends_with(['#', '?', '0'])
        || !after.starts_with(|c: char| matches!(c, '#' | '?') || c.is_ascii_digit())
    {
        return None;
    }
    let show_zero_numerator = before.ends_with('0');
    // The integer part of the mask: what precedes the numerator's placeholders.
    let int_mask = before.trim_end_matches(['#', '?', '0']).trim_end();
    // PhpSpreadsheet turns `0` and `#` denominators into `?`: only a number led by
    // 1-9 is a fixed denominator.
    let divisor_digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    let fixed_divisor = divisor_digits
        .parse::<u64>()
        .ok()
        .filter(|_| !divisor_digits.starts_with('0'));
    let vulgar = !int_mask.contains(['0', '#']) && !int_mask.starts_with('?');
    let integer = value.floor();
    let integer_text = php_float_string(integer);
    let decimals = php_float_string(value)
        .split_once('.')
        .filter(|(i, d)| {
            i.bytes().all(|b| b.is_ascii_digit()) && d.bytes().all(|b| b.is_ascii_digit())
        })
        .map(|(_, d)| d.trim_end_matches('0').to_string())
        .filter(|d| !d.is_empty());
    let (numerator, denominator) = match decimals {
        None => {
            if !vulgar && !show_zero_numerator {
                return Some(format!("{sign}{integer_text}"));
            }
            (0.0, fixed_divisor.unwrap_or(1) as f64)
        }
        Some(d) => {
            let decimal: u64 = d.parse().ok()?;
            let divisor = 10u64.checked_pow(d.len() as u32)?;
            let by = match fixed_divisor {
                Some(fd) => 100.0 / fd as f64,
                None => gcd(decimal, divisor) as f64,
            };
            // Floats, as in PHP: `# ??/16` shows 0.5 as 1/1.6.
            ((decimal as f64 / by).round(), divisor as f64 / by)
        }
    };
    let (numerator, denominator) = (php_float_string(numerator), php_float_string(denominator));
    Some(if int_mask.contains('0') {
        format!("{sign}{integer_text} {numerator}/{denominator}")
    } else if int_mask.contains('#') {
        if integer == 0.0 {
            format!("{sign}{numerator}/{denominator}")
        } else {
            format!("{sign}{integer_text} {numerator}/{denominator}")
        }
    } else if int_mask.starts_with('?') {
        let i = if integer == 0.0 {
            String::new()
        } else {
            integer_text
        };
        format!("{sign}{i} {numerator}/{denominator}")
    } else {
        let total = php_float_string(
            numerator.parse::<f64>().unwrap_or(0.0)
                + integer * denominator.parse::<f64>().unwrap_or(1.0),
        );
        format!("{sign}{total}/{denominator}")
    })
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// PhpSpreadsheet's `complexNumberFormatMask` for digits split by literals, such
/// as `000-00-0000`: the rounded number's digits fill the blocks of zeros from the
/// right; whatever is left over goes in front of the first block.
fn render_mask(section: &str, value: f64, sign: &str) -> Option<String> {
    let mask: String = section
        .chars()
        .filter(|c| !matches!(c, '"' | '*'))
        .map(|c| if matches!(c, '#' | '?') { '0' } else { c })
        .collect();
    if mask.contains(['.', ',', '[', '%']) {
        return None;
    }
    let bytes = mask.as_bytes();
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'0' {
            let start = i;
            while i < bytes.len() && bytes[i] == b'0' {
                i += 1;
            }
            blocks.push((start, i - start));
        } else {
            i += 1;
        }
    }
    if blocks.len() < 2 {
        return None;
    }
    let mut number = (value.round() as u128).to_string();
    let mut out = mask.clone();
    let mut offset = 0;
    for &(start, size) in blocks.iter().rev() {
        let take = number.len().min(size);
        let digits = number.split_off(number.len() - take);
        out.replace_range(start..start + size, &format!("{digits:0>size$}"));
        offset = start;
    }
    if !number.is_empty() {
        out.insert_str(offset, &number);
    }
    Some(format!("{sign}{out}"))
}

/// `0.00E+00` and similar: mantissa with the placeholders' decimals, exponent with
/// its sign and at least as many digits as its zeros.
fn render_scientific(section: &str, value: f64, sign: &str) -> Option<String> {
    let at = section.find(['E', 'e'])?;
    let (mantissa, rest) = (&section[..at], &section[at + 1..]);
    let exp_sign_always = rest.starts_with('+');
    if !(exp_sign_always || rest.starts_with('-')) || mantissa.contains(['"', '\\', '[']) {
        return None;
    }
    let exp_digits = rest[1..].chars().filter(|&c| c == '0').count().max(1);
    let decimals = mantissa.split_once('.').map_or(0, |(_, d)| {
        d.chars().filter(|c| matches!(c, '0' | '#' | '?')).count()
    });
    let formatted = format!("{value:.decimals$e}");
    let (m, e) = formatted.split_once('e')?;
    let e: i32 = e.parse().ok()?;
    let e_sign = if e < 0 {
        "-"
    } else if exp_sign_always {
        "+"
    } else {
        ""
    };
    Some(format!("{sign}{m}E{e_sign}{:0exp_digits$}", e.abs()))
}

/// The text PhpSpreadsheet measures for a General number: `StringHelper::
/// convertToString`, which prints a float's fraction on its own at 14 significant
/// digits, so 98765.4321 comes out as `98765.43210000000545`.
pub fn value_string(v: f64) -> String {
    let s = php_float_string(v);
    if !s
        .bytes()
        .all(|b| matches!(b, b'-' | b'+' | b'0'..=b'9' | b'.'))
    {
        return s;
    }
    let positive = v.abs();
    let floor = positive.floor();
    if floor == positive {
        return s;
    }
    let frac = php_float_string(positive - floor);
    match frac.strip_prefix("0.") {
        Some(digits) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
            let minus = if v < 0.0 { "-" } else { "" };
            format!("{minus}{}.{digits}", php_float_string(floor))
        }
        _ => s,
    }
}

/// PHP's `(string) $float` with the default `precision` of 14.
pub fn php_float_string(v: f64) -> String {
    if v.is_nan() {
        return "NAN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "INF".into() } else { "-INF".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    // 14 significant digits, then PHP's choice of fixed or scientific notation.
    let sci = format!("{v:.13e}");
    let (mantissa, exp) = sci.split_once('e').expect("formatted with e");
    let exp: i32 = exp.parse().expect("an exponent");
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    if !(-4..14).contains(&exp) {
        let m = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{mantissa}.0")
        };
        return format!("{m}E{}{}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let negative = v < 0.0;
    let point = exp + 1; // digits before the decimal point
    let mut s = String::new();
    if point <= 0 {
        s.push_str("0.");
        s.push_str(&"0".repeat((-point) as usize));
        s.push_str(&digits);
    } else if point as usize >= digits.len() {
        s.push_str(&digits);
        s.push_str(&"0".repeat(point as usize - digits.len()));
    } else {
        s.push_str(&digits[..point as usize]);
        s.push('.');
        s.push_str(&digits[point as usize..]);
    }
    if negative { format!("-{s}") } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALIBRI_12: Font<'static> = Font {
        name: "Calibri",
        size: 12.0,
    };

    fn width(text: &str, name: &str, size: f64) -> f64 {
        column_width(
            text,
            &Measure {
                font: Font { name, size },
                default_font: CALIBRI_12,
                rotation: 0,
                filter: false,
                indent: 0,
            },
        )
    }

    /// Expected values from PhpSpreadsheet 5.11 `Font::calculateColumnWidth` in
    /// approx mode, with a Calibri 12 default font, rounded to 3 places.
    #[test]
    fn widths_like_phpspreadsheet() {
        assert_eq!(width("Claimant name", "Calibri", 12.0), 16.234);
        assert_eq!(width("x", "Arial", 11.0), 2.095);
        assert_eq!(width("$1,234.50 ", "Arial", 10.0), 11.521);
        assert_eq!(width("日本", "Calibri", 11.0), 5.368);
        assert_eq!(width("one\nlonger line", "Calibri", 12.0), 13.878);
    }

    #[test]
    fn php_float_strings() {
        assert_eq!(php_float_string(0.1 + 0.2), "0.3");
        assert_eq!(php_float_string(1.0 / 3.0), "0.33333333333333");
        assert_eq!(php_float_string(1e20), "1.0E+20");
        assert_eq!(php_float_string(1234.5), "1234.5");
        assert_eq!(php_float_string(-0.000012), "-1.2E-5");
        assert_eq!(php_float_string(0.0001), "0.0001");
        assert_eq!(php_float_string(123456789012345.0), "1.2345678901234E+14");
        assert_eq!(php_float_string(12345678901234.0), "12345678901234");
    }

    #[test]
    fn number_formats() {
        let f = |v, code| format_number(v, Some(code));
        assert_eq!(f(1234.5, "$#,##0.00_-"), "$1,234.50 ");
        assert_eq!(f(-1234.5, "$#,##0.00_-"), "$-1,234.50 ");
        assert_eq!(f(0.256, "0%"), "26%");
        assert_eq!(f(0.256, "0.00%"), "25.60%");
        assert_eq!(f(1234567.0, "#,##0"), "1,234,567");
        assert_eq!(f(0.5, "#.##"), ".50");
        assert_eq!(f(-5.0, "0.00;(0.00)"), "(5.00)");
        assert_eq!(f(0.0, "0.00;(0.00);\"-\""), "-");
        assert_eq!(f(12.0, "[$€-407]#,##0.00"), "€12.00");
        assert_eq!(f(12.0, "[Red]0"), "12");
        assert_eq!(f(45823.0, "m/d/yyyy"), "6/15/2025");
        assert_eq!(f(1.5e10, "0.00E+00"), "1.50E+10");
        assert_eq!(format_number(2.5, None), "2.5");
        // PhpSpreadsheet's convertToString (expected values from PHP).
        assert_eq!(format_number(98765.4321, None), "98765.43210000000545");
        assert_eq!(format_number(-1234.5678, Some("@")), "-1234.56780000000003");
        assert_eq!(value_string(1e-7), "1.0E-7");
        assert_eq!(value_string(0.5), "0.5");
        // A fraction that itself prints in scientific notation: the plain string.
        assert_eq!(value_string(5.00000000001), "5.00000000001");
        // Not a date: measured as the number (a huge serial used to overflow).
        assert_eq!(f(3e15, "m/d/yyyy"), "3.0E+15");
    }

    #[test]
    fn wide_characters_count_twice() {
        assert_eq!(mb_strwidth("abc"), 3);
        assert_eq!(mb_strwidth("日本語"), 6);
        assert_eq!(mb_strwidth("ｶ"), 1); // halfwidth katakana
        assert_eq!(mb_strwidth("Ａ"), 2); // fullwidth A
    }

    #[test]
    fn default_fonts_in_phpspreadsheets_table() {
        // A default font with a known default column width (PhpSpreadsheet 5.11 values).
        let m = |name, size| Measure {
            font: Font { name, size },
            default_font: Font { name, size },
            rotation: 0,
            filter: false,
            indent: 0,
        };
        assert_eq!(column_width("Claimant name", &m("Calibri", 11.0)), 16.425);
        assert_eq!(column_width("x", &m("Arial", 10.0)), 2.285);
    }

    #[test]
    fn format_edge_cases_fall_back_to_the_number() {
        let f = |v, code| format_number(v, Some(code));
        // Digits split by a literal around a separator, and a date format with
        // letters the date converter doesn't map: measured as the number.
        assert_eq!(f(1234.5, "#,##0 \"x\" 0"), "1234.5");
        assert_eq!(f(45823.0, "yyyy x"), "45823");
        // A trailing backslash escapes nothing.
        assert_eq!(f(5.0, "0\\"), "5");
        assert_eq!(strip_blocks("[Red][h]:mm [$-409]x [open"), "[h]:mm x [open");
    }
}
