//! OpenSpout's XLSX string escaper (`Common\Helper\Escaper\XLSX::escape`), applied to
//! every string, formula, format code and sheet name it writes.

/// Excel's per-cell limit; OpenSpout truncates longer strings (by code point).
pub const MAX_CHARS_PER_CELL: usize = 32_767;

/// Escape `s` for an XLSX text node or double-quoted attribute:
/// 1. an existing `_xHHHH_` (uppercase hex) is protected as `_x005F_xHHHH_`;
/// 2. U+FFFE/U+FFFF and control characters other than tab, LF and CR become `_xHHHH_`;
/// 3. `&`, `"`, `<`, `>` become entities (`htmlspecialchars(ENT_COMPAT)`; `'` is kept).
pub fn escape_into(s: &str, out: &mut String) {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'_' && is_escape_sequence(&b[i..]) {
            out.push_str("_x005F");
            out.push_str(&s[i..i + 7]);
            i += 7;
            continue;
        }
        let c = s[i..].chars().next().expect("char boundary");
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{FFFE}' => out.push_str("_xFFFE_"),
            '\u{FFFF}' => out.push_str("_xFFFF_"),
            '\u{00}'..='\u{08}' | '\u{0B}' | '\u{0C}' | '\u{0E}'..='\u{1F}' => {
                out.push_str(&format!("_x{:04X}_", c as u32));
            }
            c => out.push(c),
        }
        i += c.len_utf8();
    }
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    escape_into(s, &mut out);
    out
}

/// `_(x[\dA-F]{4})_` at the start of `b`.
fn is_escape_sequence(b: &[u8]) -> bool {
    b.len() >= 7
        && b[1] == b'x'
        && b[2..6]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'A'..=b'F').contains(c))
        && b[6] == b'_'
}

/// Truncate to Excel's limit like `mb_substr($s, 0, 32767)`.
pub fn truncate_for_cell(s: &str) -> &str {
    match s.char_indices().nth(MAX_CHARS_PER_CELL) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values from OpenSpout's escaper (verified with PHP).
    #[test]
    fn escapes_like_openspout() {
        assert_eq!(escape("<&>\"'"), "&lt;&amp;&gt;&quot;'");
        assert_eq!(escape("a\u{1}b\tc\nd\re"), "a_x0001_b\tc\nd\re");
        assert_eq!(escape("_x0041_ and _x00ff_"), "_x005F_x0041_ and _x00ff_");
        assert_eq!(escape("\u{FFFE}"), "_xFFFE_");
        assert_eq!(escape("café 🚀"), "café 🚀");
    }

    #[test]
    fn truncates_by_code_point() {
        let long = "é".repeat(MAX_CHARS_PER_CELL + 5);
        assert_eq!(truncate_for_cell(&long).chars().count(), MAX_CHARS_PER_CELL);
        assert_eq!(truncate_for_cell("short"), "short");
    }
}
