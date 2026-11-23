//! A port of PHP's `php_fgetcsv()` (php-src `ext/standard/file.c`, PHP 8.5), which
//! OpenSpout's CSV reader is built on. Ported line by line so messy input parses
//! exactly as it does in PHP: leading whitespace before a quote, text after a
//! closing quote, quoted fields spanning lines, unterminated quotes at EOF, and
//! stripping of a single trailing line ending.
//!
//! Like PHP in the default "C" locale (and for any UTF-8 input), bytes are
//! processed one at a time; delimiter and enclosure are single ASCII bytes.

use std::io::{self, BufRead};

/// Reads physical lines the way `php_stream_get_line()` does: up to and
/// including `\n` (a bare `\r` is not a line ending), or the rest at EOF.
pub struct LineSource<R> {
    inner: R,
    /// 1-based number of the last line read.
    pub line: u64,
}

impl<R: BufRead> LineSource<R> {
    pub fn new(inner: R) -> Self {
        LineSource { inner, line: 0 }
    }

    /// `None` at end of input.
    pub fn next_line(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut buf = Vec::new();
        if self.inner.read_until(b'\n', &mut buf)? == 0 {
            return Ok(None);
        }
        self.line += 1;
        Ok(Some(buf))
    }
}

/// PHP's `memcpy(tptr, buf + from, to - from)`: like `at()`, the byte just past
/// the end of the line is the string's NUL terminator.
fn copy(field: &mut Vec<u8>, buf: &[u8], from: usize, to: usize) {
    let end = to.min(buf.len());
    if from < end {
        field.extend_from_slice(&buf[from..end]);
    }
    field.extend(std::iter::repeat_n(0, to.saturating_sub(end.max(from))));
}

/// `isspace()` in the C locale.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// `php_fgetcsv_lookup_trailing_spaces`: the length without one trailing
/// `\r\n`, `\n` or `\r`.
///
/// PHP walks the line with `mblen()` and remembers the first byte of the last two
/// *characters*; an invalid multibyte sequence is stepped over without being
/// remembered. So when the line ends in `\r` followed by invalid UTF-8, PHP still
/// sees `\r` as the last character and cuts that many bytes off the end, which
/// are then the invalid bytes. This matches PHP running in a UTF-8 `LC_CTYPE`.
fn trim_line_end(s: &[u8]) -> usize {
    let len = s.len();
    // Fast path: an ASCII byte is always a character of its own, so when the last
    // byte (and, for `\n`, the one before it) is ASCII, no scan is needed.
    match s {
        [] => return 0,
        [.., last] if *last >= 0x80 => {}
        [.., b'\r'] => return len - 1,
        [.., b'\r', b'\n'] => return len - 2,
        [.., prev, b'\n'] if *prev < 0x80 => return len - 1,
        [b'\n'] => return len - 1,
        [.., b'\n'] => {}
        _ => return len,
    }
    let mut last = [0u8; 2];
    let mut i = 0;
    while i < len {
        match utf8_char_len(&s[i..]) {
            Some(w) => {
                last = [last[1], s[i]];
                i += w;
            }
            None => i += 1, // invalid: skipped, not remembered
        }
    }
    match last {
        [b'\r', b'\n'] => len - 2,
        [_, b'\n' | b'\r'] => len - 1,
        _ => len,
    }
}

/// The length of the valid UTF-8 character starting `s`, or `None` (mblen() < 0).
fn utf8_char_len(s: &[u8]) -> Option<usize> {
    let width = match s[0] {
        0x00..=0x7F => return Some(1),
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    let candidate = s.get(..width)?;
    std::str::from_utf8(candidate).ok().map(|_| width)
}

/// Parse one CSV record starting with `first_line`, pulling continuation lines
/// from `lines` for quoted fields that span lines.
///
/// Returns `None` for a blank line (PHP's `fgetcsv()` returns `[null]` there).
pub fn parse_record<R: BufRead>(
    first_line: Vec<u8>,
    lines: &mut LineSource<R>,
    delimiter: u8,
    enclosure: u8,
    escape: Option<u8>,
) -> io::Result<Option<Vec<Vec<u8>>>> {
    let mut buf = first_line;
    let mut limit = trim_line_end(&buf);
    let mut line_end = limit;
    let mut bptr: usize = 0;
    let mut fields: Vec<Vec<u8>> = Vec::new();
    let mut first_field = true;
    // `*ptr` past the end reads PHP's NUL terminator.
    let at = |buf: &[u8], i: usize| buf.get(i).copied().unwrap_or(0);

    loop {
        let mut field: Vec<u8> = Vec::new();
        let mut inc_len: usize = usize::from(bptr < limit);

        // 1. Skip leading whitespace, but only if a quoted field follows it.
        if inc_len == 1 {
            let mut tmp = bptr;
            while at(&buf, tmp) != delimiter && is_space(at(&buf, tmp)) {
                tmp += 1;
            }
            if at(&buf, tmp) == enclosure && tmp < limit {
                bptr = tmp;
            }
        }

        if first_field && bptr == line_end {
            return Ok(None);
        }
        first_field = false;

        if inc_len != 0 && at(&buf, bptr) == enclosure {
            // 2A. Enclosed field.
            let mut state = 0u8;
            bptr += 1;
            let mut hunk = bptr;
            loop {
                if inc_len == 0 {
                    match state {
                        2 => {
                            copy(&mut field, &buf, hunk, bptr - 1);
                            hunk = bptr;
                            break;
                        }
                        _ => {
                            if state == 1 {
                                copy(&mut field, &buf, hunk, bptr);
                                hunk = bptr;
                            }
                            if hunk != line_end {
                                copy(&mut field, &buf, hunk, bptr);
                                hunk = bptr;
                            }
                            // The line break is part of the field.
                            field.extend_from_slice(&buf[line_end..]);
                            match lines.next_line()? {
                                None => {
                                    // Unterminated enclosure: everything to EOF is the field.
                                    if bptr > limit {
                                        if hunk == bptr {
                                            hunk -= 1;
                                        }
                                        bptr -= 1;
                                    }
                                    break;
                                }
                                Some(next) => {
                                    buf = next;
                                    bptr = 0;
                                    hunk = 0;
                                    limit = trim_line_end(&buf);
                                    line_end = limit;
                                    state = 0;
                                }
                            }
                        }
                    }
                } else {
                    match state {
                        1 => {
                            // Escaped character.
                            bptr += 1;
                            state = 0;
                        }
                        2 => {
                            if at(&buf, bptr) != enclosure {
                                // The closing enclosure.
                                copy(&mut field, &buf, hunk, bptr - 1);
                                hunk = bptr;
                                break;
                            }
                            // A doubled enclosure: keep one.
                            copy(&mut field, &buf, hunk, bptr);
                            bptr += 1;
                            hunk = bptr;
                            state = 0;
                        }
                        // A field opened at the very end of the input reads the
                        // NUL terminator here, as PHP does.
                        _ => {
                            if at(&buf, bptr) == enclosure {
                                state = 2;
                            } else if escape == Some(at(&buf, bptr)) {
                                state = 1;
                            }
                            bptr += 1;
                        }
                    }
                }
                inc_len = usize::from(bptr < limit);
            }

            // Text after the closing enclosure, up to the delimiter, is kept.
            loop {
                if inc_len == 0 || buf[bptr] == delimiter {
                    break;
                }
                bptr += 1;
                inc_len = usize::from(bptr < limit);
            }
            copy(&mut field, &buf, hunk.min(bptr), bptr);
            bptr += inc_len;
        } else {
            // 2B. Plain field.
            let hunk = bptr;
            loop {
                if inc_len == 0 || buf[bptr] == delimiter {
                    break;
                }
                bptr += 1;
                inc_len = usize::from(bptr < limit);
            }
            field.extend_from_slice(&buf[hunk..bptr]);
            let len = trim_line_end(&field);
            field.truncate(len);
            if at(&buf, bptr) == delimiter {
                bptr += 1;
            }
        }

        fields.push(field);
        if inc_len == 0 {
            return Ok(Some(fields));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a whole input with PHP-like defaults; `None` rows are blank lines.
    fn parse_all(input: &str) -> Vec<Option<Vec<String>>> {
        let mut src = LineSource::new(input.as_bytes());
        let mut out = vec![];
        while let Some(line) = src.next_line().unwrap() {
            let rec = parse_record(line, &mut src, b',', b'"', None).unwrap();
            out.push(rec.map(|f| {
                f.into_iter()
                    .map(|b| String::from_utf8(b).unwrap())
                    .collect()
            }));
        }
        out
    }

    fn row(cells: &[&str]) -> Option<Vec<String>> {
        Some(cells.iter().map(|s| s.to_string()).collect())
    }

    // Expected values below were produced by PHP 8.5's str_getcsv/fgetcsv.
    #[test]
    fn basics() {
        assert_eq!(
            parse_all("a,b,c\n1,2,3\n"),
            [row(&["a", "b", "c"]), row(&["1", "2", "3"])]
        );
        assert_eq!(
            parse_all("a,b\r\nc,d"),
            [row(&["a", "b"]), row(&["c", "d"])]
        );
        assert_eq!(parse_all("a,,\n"), [row(&["a", "", ""])]);
        assert_eq!(parse_all("\n"), [None]);
        assert_eq!(parse_all("\r\n"), [None]);
        assert_eq!(parse_all(" \n"), [row(&[" "])]);
        assert_eq!(parse_all("\"\"\n"), [row(&[""])]);
    }

    #[test]
    fn quoting() {
        assert_eq!(parse_all("\"a,b\",c\n"), [row(&["a,b", "c"])]);
        assert_eq!(
            parse_all("\"say \"\"hi\"\"\",x\n"),
            [row(&["say \"hi\"", "x"])]
        );
        assert_eq!(parse_all("  \"a\",b\n"), [row(&["a", "b"])]);
        assert_eq!(parse_all("\"a\"b,c\n"), [row(&["ab", "c"])]);
        assert_eq!(parse_all("\"a\" ,c\n"), [row(&["a ", "c"])]);
        assert_eq!(parse_all("x\"y\",z\n"), [row(&["x\"y\"", "z"])]);
    }

    #[test]
    fn multiline_fields() {
        assert_eq!(
            parse_all("\"a\nb\",c\nd\n"),
            [row(&["a\nb", "c"]), row(&["d"])]
        );
        assert_eq!(parse_all("\"a\r\nb\",c\r\n"), [row(&["a\r\nb", "c"])]);
        // Unterminated at EOF: the rest of the input, line break included.
        assert_eq!(parse_all("x,\"open\nrest\n"), [row(&["x", "open\nrest\n"])]);
    }

    #[test]
    fn bare_cr_is_not_a_line_break() {
        assert_eq!(parse_all("a,b\rc,d\n"), [row(&["a", "b\rc", "d"])]);
    }

    #[test]
    fn enclosure_edge_cases_like_php() {
        assert_eq!(parse_all("\"a\"\"b\",c"), [row(&["a\"b", "c"])]);
        assert_eq!(parse_all("\"x\" y,z"), [row(&["x y", "z"])]);
        assert_eq!(parse_all("\"ab\"c\"\n"), [row(&["abc\""])]);
        assert_eq!(parse_all("\"q\"\"\n\""), [row(&["q\"\n"])]);
        assert_eq!(parse_all("\"unterminated"), [row(&["unterminated"])]);
    }

    /// Found by the differential test against PHP: input ending in an opening
    /// enclosure (no line break after it) read past the end of the buffer.
    #[test]
    fn enclosure_at_end_of_input() {
        assert_eq!(parse_all("\""), [row(&[""])]);
        assert_eq!(parse_all("\"\",\""), [row(&["", ""])]);
        assert_eq!(parse_all("\"\"\"\",\""), [row(&["\"", ""])]);
        assert_eq!(parse_all("\"\"\n\""), [row(&[""]), row(&[""])]);
    }

    /// Found by the differential test against PHP 8.5 (LC_CTYPE=en_US.UTF-8): when
    /// looking for the trailing line ending, PHP skips invalid multibyte sequences
    /// without noting them, so a final `\r` followed by invalid bytes still counts
    /// as the line ending and the last byte(s) are cut instead.
    #[test]
    fn line_end_lookup_skips_invalid_multibyte_like_php() {
        assert_eq!(trim_line_end(b"\r\xFF"), 1);
        assert_eq!(trim_line_end(b"a\r\xFF"), 2);
        assert_eq!(trim_line_end(b"\"a\"\r\xFF"), 4);
        assert_eq!(trim_line_end(b"a\r\n\xFF\xFE"), 3);
        // A valid multibyte character is a character: no line ending after it.
        assert_eq!(trim_line_end(b"\r\xC3\xA9"), 3);
        assert_eq!(trim_line_end(b"x\xFF\r"), 2);
        assert_eq!(trim_line_end(b"a\r\xFFb"), 4);
        let rows = |input: &[u8]| {
            let mut src = LineSource::new(input);
            let line = src.next_line().unwrap().unwrap();
            parse_record(line, &mut src, b',', b'"', None).unwrap()
        };
        assert_eq!(rows(b"\r\xFF"), Some(vec![vec![]]));
        assert_eq!(rows(b"a\r\xFF"), Some(vec![b"a".to_vec()]));
        assert_eq!(rows(b"\"a\"\r\xFF"), Some(vec![b"a\r".to_vec()]));
    }

    /// With a NUL escape character, an enclosure at the very end of the input reads
    /// PHP's string terminator as an escape and copies it into the field. Expected
    /// values from PHP 8.5's fgetcsv(); this used to slice past the buffer.
    #[test]
    fn nul_escape_at_end_of_input_copies_the_terminator() {
        let parse = |input: &[u8]| {
            let mut src = LineSource::new(input);
            let line = src.next_line().unwrap().unwrap();
            parse_record(line, &mut src, b',', b'"', Some(0)).unwrap()
        };
        assert_eq!(parse(b"\""), Some(vec![b"\0".to_vec()]));
        assert_eq!(parse(b"a,\""), Some(vec![b"a".to_vec(), b"\0".to_vec()]));
        assert_eq!(parse(b"\"x"), Some(vec![b"x".to_vec()]));
        assert_eq!(parse(b"\"a\0b\""), Some(vec![b"a\0b".to_vec()]));
    }
}
