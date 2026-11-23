//! Shared-formula expansion.
//!
//! Excel stores a filled-down formula once, on the "master" cell
//! (`<f t="shared" ref="D5:D7" si="0">B5*2</f>`); the other cells only carry
//! `<f t="shared" si="0"/>`. OpenSpout returns `"="` for those children, which is a bug.
//! We rebuild each child's formula by shifting the master's relative references
//! by the row/column distance, as Excel does: `B5*2` → `B6*2` one row down.
//!
//! The shifter is a small lexer. It copies string literals, quoted sheet names and
//! `[...]` sections verbatim, leaves function names, defined names and numbers
//! alone, and only rewrites A1-style references and whole-row/column ranges.
//! References pushed off the grid become `#REF!`, as in Excel.

const MAX_COL: i64 = 16_384; // XFD
const MAX_ROW: i64 = 1_048_576;

/// Shift the relative references in `formula` by `drow` rows and `dcol` columns.
pub fn shift(formula: &str, drow: i64, dcol: i64) -> String {
    if drow == 0 && dcol == 0 {
        return formula.to_string();
    }
    let s = formula.as_bytes();
    let mut out = String::with_capacity(formula.len() + 8);
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'"' => i = copy_quoted(formula, i, b'"', &mut out),
            b'\'' => i = copy_quoted(formula, i, b'\'', &mut out),
            b'[' => i = copy_brackets(formula, i, &mut out),
            b if is_token_byte(b) => {
                let end = i + s[i..].iter().take_while(|&&b| is_token_byte(b)).count();
                i = shift_token(formula, i, end, drow, dcol, &mut out);
            }
            _ => {
                // Multi-byte UTF-8 (e.g. in names) is copied a whole char at a time.
                let ch = formula[i..].chars().next().expect("in bounds");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'$' | b'_' | b'.' | b'\\')
}

/// Copy a quoted run (`"…"` or `'…'`, with doubled quotes as escapes). Returns the next index.
fn copy_quoted(f: &str, start: usize, quote: u8, out: &mut String) -> usize {
    let s = f.as_bytes();
    let mut i = start + 1;
    while i < s.len() {
        if s[i] == quote {
            if s.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            i += 1;
            break;
        }
        i += 1;
    }
    out.push_str(&f[start..i.min(s.len())]);
    i.min(s.len())
}

/// Copy a `[...]` section (external workbook index or structured reference), with nesting.
fn copy_brackets(f: &str, start: usize, out: &mut String) -> usize {
    let s = f.as_bytes();
    let mut depth = 0;
    let mut i = start;
    while i < s.len() {
        match s[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    i += 1;
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out.push_str(&f[start..i]);
    i
}

/// A parsed `$?COL$?ROW`, `$?COL` or `$?ROW` component.
#[derive(Clone, Copy)]
struct Part {
    col: Option<(bool, i64)>, // (absolute, 1-based)
    row: Option<(bool, i64)>,
}

fn parse_part(tok: &str) -> Option<Part> {
    let s = tok.as_bytes();
    let mut i = 0;
    let col_abs = s.first() == Some(&b'$');
    if col_abs {
        i += 1;
    }
    let letters = s[i..]
        .iter()
        .take_while(|b| b.is_ascii_alphabetic())
        .count();
    let col = if letters > 0 {
        if letters > 3 {
            return None;
        }
        let n = s[i..i + letters].iter().fold(0i64, |acc, b| {
            acc * 26 + i64::from(b.to_ascii_uppercase() - b'A' + 1)
        });
        i += letters;
        Some((col_abs, n))
    } else {
        None
    };
    let row_abs = s.get(i) == Some(&b'$');
    if row_abs {
        if col.is_none() && col_abs {
            return None; // "$$5"
        }
        i += 1;
    }
    let digits = s[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    let row = if digits > 0 {
        let n: i64 = tok[i..i + digits].parse().ok()?;
        i += digits;
        Some((row_abs || (col.is_none() && col_abs), n))
    } else {
        if row_abs {
            return None;
        }
        None
    };
    if i != s.len() || (col.is_none() && row.is_none()) {
        return None;
    }
    if col.is_some_and(|(_, c)| c > MAX_COL) || row.is_some_and(|(_, r)| r == 0 || r > MAX_ROW) {
        return None;
    }
    Some(Part { col, row })
}

/// Render a shifted part, or `None` if it falls off the grid.
fn render(p: Part, drow: i64, dcol: i64) -> Option<String> {
    let mut s = String::new();
    if let Some((abs, c)) = p.col {
        let c = if abs { c } else { c + dcol };
        if !(1..=MAX_COL).contains(&c) {
            return None;
        }
        if abs {
            s.push('$');
        }
        s.push_str(&col_letters(c));
    }
    if let Some((abs, r)) = p.row {
        let r = if abs { r } else { r + drow };
        if !(1..=MAX_ROW).contains(&r) {
            return None;
        }
        if abs {
            s.push('$');
        }
        s.push_str(&r.to_string());
    }
    Some(s)
}

fn col_letters(mut n: i64) -> String {
    let mut buf = Vec::with_capacity(3);
    while n > 0 {
        let rem = (n - 1) % 26;
        buf.push(b'A' + rem as u8);
        n = (n - 1) / 26;
    }
    buf.reverse();
    String::from_utf8(buf).expect("ASCII")
}

/// Handle one identifier-like token `s[start..end]`, possibly extending it into a
/// range (`A1:B2`, `A:B`, `1:5`). Returns the index after what was consumed.
fn shift_token(f: &str, start: usize, end: usize, drow: i64, dcol: i64, out: &mut String) -> usize {
    let s = f.as_bytes();
    let tok = &f[start..end];
    let next = s.get(end).copied();

    // Function names (`SUM(`, `LOG10(`), sheet prefixes (`Sheet1!`) and tokens glued to a
    // preceding identifier aren't references.
    let glued = start > 0 && (s[start - 1].is_ascii_alphanumeric() || s[start - 1] == b'_');
    if glued || next == Some(b'(') || next == Some(b'!') {
        out.push_str(tok);
        return end;
    }

    let Some(first) = parse_part(tok) else {
        out.push_str(tok);
        return end;
    };

    // Range: `first:second`, where both sides are the same kind of part.
    if next == Some(b':') {
        let second_start = end + 1;
        let second_end = second_start
            + s[second_start..]
                .iter()
                .take_while(|&&b| is_token_byte(b))
                .count();
        if let Some(second) = parse_part(&f[second_start..second_end]) {
            let same_kind = first.col.is_some() == second.col.is_some()
                && first.row.is_some() == second.row.is_some();
            if same_kind && s.get(second_end) != Some(&b'(') {
                match (render(first, drow, dcol), render(second, drow, dcol)) {
                    (Some(a), Some(b)) => {
                        out.push_str(&a);
                        out.push(':');
                        out.push_str(&b);
                    }
                    _ => out.push_str("#REF!"),
                }
                return second_end;
            }
        }
    }

    // A lone column (`A`) or row (`5`) isn't a reference outside a range:
    // `A` could be a defined name, `5` is a number.
    if first.col.is_none() || first.row.is_none() {
        out.push_str(tok);
        return end;
    }
    match render(first, drow, dcol) {
        Some(r) => out.push_str(&r),
        None => out.push_str("#REF!"),
    }
    end
}

#[cfg(test)]
mod tests {
    use super::shift;

    #[test]
    fn relative_and_absolute() {
        assert_eq!(shift("B5*2", 1, 0), "B6*2");
        assert_eq!(shift("$B$5*B5", 2, 1), "$B$5*C7");
        assert_eq!(shift("$B5+B$5", 1, 1), "$B6+C$5");
        assert_eq!(shift("b5", 1, 0), "B6");
    }

    #[test]
    fn ranges() {
        assert_eq!(shift("SUM(A1:B2)", 1, 0), "SUM(A2:B3)");
        assert_eq!(shift("SUM(A:A)", 0, 1), "SUM(B:B)");
        assert_eq!(shift("SUM(1:3)", 2, 0), "SUM(3:5)");
        assert_eq!(shift("SUM($A:B)", 0, 1), "SUM($A:C)");
    }

    #[test]
    fn leaves_non_references_alone() {
        assert_eq!(shift("LOG10(A1)", 1, 0), "LOG10(A2)");
        assert_eq!(shift("\"A1\"&A1", 1, 0), "\"A1\"&A2");
        assert_eq!(shift("'My A1'!A1", 1, 0), "'My A1'!A2");
        assert_eq!(shift("Sheet1!A1+1.5E+3", 1, 0), "Sheet1!A2+1.5E+3");
        assert_eq!(
            shift("Table1[[#This Row],[A1]]", 1, 0),
            "Table1[[#This Row],[A1]]"
        );
        assert_eq!(shift("[1]Sheet1!B2", 1, 0), "[1]Sheet1!B3");
        assert_eq!(shift("TaxRate*A1", 1, 0), "TaxRate*A2");
        assert_eq!(shift("A1*5", 1, 0), "A2*5");
        assert_eq!(shift("IF(A1>0,TRUE,FALSE)", 1, 0), "IF(A2>0,TRUE,FALSE)");
    }

    #[test]
    fn off_grid_becomes_ref_error() {
        assert_eq!(shift("A1", -1, 0), "#REF!");
        assert_eq!(shift("A1:B1", 0, -1), "#REF!");
        assert_eq!(shift("XFD1", 0, 1), "#REF!");
    }

    #[test]
    fn no_shift_is_identity() {
        assert_eq!(shift("anything(A1)", 0, 0), "anything(A1)");
    }

    #[test]
    fn tokens_that_are_not_references_are_copied() {
        // Doubled quotes stay inside the string literal.
        assert_eq!(shift("\"a\"\"A1\"&A1", 1, 0), "\"a\"\"A1\"&A2");
        // `$$5` and `A$` are not references.
        assert_eq!(shift("$$5+A$", 1, 1), "$$5+A$");
        // A range of mixed kinds is two separate tokens; a lone column isn't shifted.
        assert_eq!(shift("A1:B", 1, 0), "A2:B");
        // `A1:` followed by something that isn't a reference: only A1 moves.
        assert_eq!(shift("A1:$$5", 1, 0), "A2:$$5");
    }
}
