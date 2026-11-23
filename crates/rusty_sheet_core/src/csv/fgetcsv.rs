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

/// `isspace()` in the C locale.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// `php_fgetcsv_lookup_trailing_spaces`: the length without one trailing
/// `\r\n`, `\n` or `\r`.
fn trim_line_end(s: &[u8]) -> usize {
    match s {
        [.., b'\r', b'\n'] => s.len() - 2,
        [.., b'\n'] | [.., b'\r'] => s.len() - 1,
        _ => s.len(),
    }
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
                            field.extend_from_slice(&buf[hunk..bptr - 1]);
                            hunk = bptr;
                            break;
                        }
                        _ => {
                            if state == 1 {
                                field.extend_from_slice(&buf[hunk..bptr]);
                                hunk = bptr;
                            }
                            if hunk != line_end {
                                field.extend_from_slice(&buf[hunk..bptr]);
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
                            if buf[bptr] != enclosure {
                                // The closing enclosure.
                                field.extend_from_slice(&buf[hunk..bptr - 1]);
                                hunk = bptr;
                                break;
                            }
                            // A doubled enclosure: keep one.
                            field.extend_from_slice(&buf[hunk..bptr]);
                            bptr += 1;
                            hunk = bptr;
                            state = 0;
                        }
                        _ => {
                            if buf[bptr] == enclosure {
                                state = 2;
                            } else if escape == Some(buf[bptr]) {
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
            field.extend_from_slice(&buf[hunk.min(bptr)..bptr]);
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
}
