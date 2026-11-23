//! OpenSpout's reader-side `Escaper\XLSX::unescape`, ported literally:
//!
//! ```php
//! foreach (controlCharactersEscapingMap as $escaped => $char) {   // 0x00..0x1F (not \t \n \r), FFFE, FFFF
//!     $s = preg_replace("/(?<!_x005F)({$escaped})/", $char, $s);
//! }
//! $s = preg_replace('/_x005F(_x[\dA-F]{4}_)/', '$1', $s);
//! ```
//!
//! The order matters for overlapping sequences (`_x0002_x0001_` → `_x0002` + U+0001),
//! so this runs one pass per escape, exactly like the PHP. Other `_xHHHH_` sequences
//! (e.g. `_x000D_`) are left alone, as in OpenSpout.

use std::borrow::Cow;

/// The escapes OpenSpout decodes, in its map order.
fn escapes() -> impl Iterator<Item = (String, char)> {
    (0x00u32..=0x1F)
        .filter(|c| !matches!(c, 0x09 | 0x0A | 0x0D))
        .chain([0xFFFE, 0xFFFF])
        .map(|c| {
            (
                format!("_x{c:04X}_"),
                char::from_u32(c).expect("valid scalar"),
            )
        })
}

pub fn unescape(s: &str) -> Cow<'_, str> {
    if !s.contains("_x") {
        return Cow::Borrowed(s);
    }
    let mut current = s.to_string();
    for (key, ch) in escapes() {
        if current.contains(&key) {
            current = replace_unprotected(&current, &key, ch);
        }
    }
    Cow::Owned(unwrap_protected(&current))
}

/// `preg_replace("/(?<!_x005F)(KEY)/", CHAR, $s)`: non-overlapping, left to right;
/// a protected occurrence is skipped one character at a time (a match may start at
/// its closing underscore).
fn replace_unprotected(s: &str, key: &str, ch: char) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(pos) = s[i..].find(key) {
        let at = i + pos;
        out.push_str(&s[i..at]);
        if s[..at].ends_with("_x005F") {
            out.push('_');
            i = at + 1;
        } else {
            out.push(ch);
            i = at + key.len();
        }
    }
    out.push_str(&s[i..]);
    out
}

/// `preg_replace('/_x005F(_x[\dA-F]{4}_)/', '$1', $s)`: each match is consumed whole.
fn unwrap_protected(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"_x005F") && is_hex_escape(&b[i + 6..]) {
            out.push_str(&s[i + 6..i + 13]);
            i += 13;
            continue;
        }
        let c = s[i..].chars().next().expect("char boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// `_xHHHH_` (uppercase hex) at the start of `b`.
fn is_hex_escape(b: &[u8]) -> bool {
    b.len() >= 7
        && b[0] == b'_'
        && b[1] == b'x'
        && b[2..6]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'A'..=b'F').contains(c))
        && b[6] == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values produced by OpenSpout 5.15's Escaper\XLSX::unescape.
    #[test]
    fn unescapes_like_openspout() {
        for (input, expected) in [
            ("a_x0001_b", "a\u{1}b"),
            ("line1_x000D_\nline2", "line1_x000D_\nline2"),
            ("_x005F_x0001_", "_x0001_"),
            ("_x001a_ _x001A_", "_x001a_ \u{1A}"),
            ("_xFFFE_", "\u{FFFE}"),
            ("_x0041_", "_x0041_"),
            ("plain", "plain"),
            // Review regressions: protected escapes and overlapping sequences.
            ("_x005F_x005F_x0041_", "_x005F_x0041_"),
            ("_x0002_x0001_", "_x0002\u{1}"),
            ("_x005F_x0001_x0001_", "_x005F_x0001\u{1}"),
            ("_x0001__x0002_", "\u{1}\u{2}"),
            ("x_x005F_x005F_x0001_", "x_x005F_x0001_"),
        ] {
            assert_eq!(unescape(input), expected, "{input:?}");
        }
    }

    #[test]
    fn round_trips_the_writer_escape() {
        use crate::writer::escape_for_tests as escape;
        for s in [
            "_x005F_x0041_",
            "_x0041_",
            "a\u{1}b",
            "_x0001_",
            "\u{FFFE}x",
        ] {
            // The writer's output is XML-escaped too; none of these contain & < > ".
            assert_eq!(unescape(&escape(s)), s, "{s:?}");
        }
    }
}
