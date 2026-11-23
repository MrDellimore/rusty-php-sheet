//! `xl/styles.xml` → for each cell style (`cellXfs` index), how numbers in that
//! style are materialised. Classified once up front so the row loop does a single
//! `Vec` lookup per cell.

use std::collections::HashMap;
use std::io::BufReader;

use quick_xml::events::Event;

use super::numfmt;
use super::package::Package;
use super::xml;
use crate::Result;
use crate::value::{DateKind, NumFmtId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellFormat {
    General,
    Date(DateKind, NumFmtId),
}

#[derive(Debug, Default)]
pub struct Styles {
    formats: Vec<CellFormat>,
    custom_codes: HashMap<u32, String>,
}

#[derive(Debug)]
struct Xf {
    num_fmt_id: Option<i64>,
    apply_number_format: Option<bool>,
}

impl Styles {
    /// Classify every cell style, following OpenSpout's `formatNumericCellValue`:
    /// 1. if the style's format code is a duration (`[h]:mm:ss`) → duration. This
    ///    applies to style 0 too and ignores `applyNumberFormat`;
    /// 2. else, for styles other than 0 with `applyNumberFormat` not false and a
    ///    `numFmtId` that is a built-in date id or a custom date code → date;
    /// 3. else a plain number.
    pub fn read(pkg: &Package, part: &str) -> Result<Self> {
        let (xfs, custom_codes) = parse(pkg, part)?;
        let mut styles = Styles {
            formats: Vec::with_capacity(xfs.len()),
            custom_codes,
        };
        for (style_id, xf) in xfs.iter().enumerate() {
            let format = styles.classify(style_id, xf);
            styles.formats.push(format);
        }
        Ok(styles)
    }

    fn classify(&self, style_id: usize, xf: &Xf) -> CellFormat {
        let Some(id) = xf.num_fmt_id else {
            return CellFormat::General;
        };
        let Ok(id) = u32::try_from(id) else {
            return CellFormat::General;
        };
        let fmt = NumFmtId(id);
        if numfmt::is_duration_format(self.format_code(fmt)) {
            return CellFormat::Date(DateKind::Duration, fmt);
        }
        if style_id == 0 || xf.apply_number_format == Some(false) {
            return CellFormat::General;
        }
        let is_date = numfmt::builtin_date_format(id).is_some()
            || self
                .custom_codes
                .get(&id)
                .is_some_and(|code| numfmt::is_date_format_code(code));
        if is_date {
            CellFormat::Date(DateKind::DateTime, fmt)
        } else {
            CellFormat::General
        }
    }

    #[inline]
    pub fn cell_format(&self, style_id: u32) -> CellFormat {
        self.formats
            .get(style_id as usize)
            .copied()
            .unwrap_or(CellFormat::General)
    }

    /// The format code OpenSpout associates with a number format id:
    /// its own built-in date mapping first, then the workbook's custom codes, else "".
    pub fn format_code(&self, fmt: NumFmtId) -> &str {
        numfmt::builtin_date_format(fmt.0)
            .or_else(|| self.custom_codes.get(&fmt.0).map(String::as_str))
            .unwrap_or("")
    }
}

fn parse(pkg: &Package, part: &str) -> Result<(Vec<Xf>, HashMap<u32, String>)> {
    let bytes = pkg.read_part(part)?;
    let mut reader = xml::reader(BufReader::new(bytes.as_slice()));
    let mut buf = Vec::new();
    let mut xfs = Vec::new();
    let mut codes = HashMap::new();
    let mut in_num_fmts = false;
    let mut in_cell_xfs = false;

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(xml::xml_error(part))?
        {
            Event::Start(e) | Event::Empty(e) => match xml::local_name(&e) {
                "numFmts" => in_num_fmts = true,
                "cellXfs" => in_cell_xfs = true,
                "numFmt" if in_num_fmts => {
                    let id = xml::attr(&e, "numFmtId").map(|v| php_int(&v));
                    let code = xml::attr_unescaped(&e, "formatCode");
                    if let (Some(id), Some(code)) = (id, code) {
                        if let Ok(id) = u32::try_from(id) {
                            codes.insert(id, code);
                        }
                    }
                }
                "xf" if in_cell_xfs => xfs.push(Xf {
                    num_fmt_id: xml::attr(&e, "numFmtId").map(|v| php_int(&v)),
                    apply_number_format: xml::attr(&e, "applyNumberFormat").map(|v| php_bool(&v)),
                }),
                _ => {}
            },
            Event::End(e) => match xml::end_local_name(e.name().0) {
                "numFmts" => in_num_fmts = false,
                "cellXfs" => in_cell_xfs = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok((xfs, codes))
}

/// PHP `(int) "…"` for the attribute values we care about: optional leading
/// whitespace and sign, then digits; anything else stops the parse (→ 0 if none).
pub fn php_int(s: &str) -> i64 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0B', '\x0C']);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut n: i64 = 0;
    for b in digits.bytes().take_while(u8::is_ascii_digit) {
        n = n.saturating_mul(10).saturating_add(i64::from(b - b'0'));
    }
    if neg { -n } else { n }
}

/// PHP `(bool) "…"`: only "" and "0" are false.
pub fn php_bool(s: &str) -> bool {
    !(s.is_empty() || s == "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles(xfs: Vec<(Option<i64>, Option<bool>)>, codes: &[(u32, &str)]) -> Styles {
        let mut s = Styles {
            formats: vec![],
            custom_codes: codes.iter().map(|(i, c)| (*i, c.to_string())).collect(),
        };
        let xfs: Vec<Xf> = xfs
            .into_iter()
            .map(|(num_fmt_id, apply_number_format)| Xf {
                num_fmt_id,
                apply_number_format,
            })
            .collect();
        for (i, xf) in xfs.iter().enumerate() {
            let f = s.classify(i, xf);
            s.formats.push(f);
        }
        s
    }

    #[test]
    fn classification_matches_openspout() {
        let s = styles(
            vec![
                (Some(14), None),        // 0: style 0 is never a date…
                (Some(14), None),        // 1: built-in date
                (Some(14), Some(false)), // 2: applyNumberFormat="0"
                (Some(164), None),       // 3: custom date
                (Some(165), None),       // 4: custom non-date
                (Some(46), Some(false)), // 5: duration ignores applyNumberFormat
                (None, None),            // 6: no numFmtId
                (Some(4), None),         // 7: built-in number
            ],
            &[(164, "yyyy-mm-dd"), (165, "0.00")],
        );
        use CellFormat::*;
        use DateKind::*;
        assert_eq!(s.cell_format(0), General);
        assert_eq!(s.cell_format(1), Date(DateTime, NumFmtId(14)));
        assert_eq!(s.cell_format(2), General);
        assert_eq!(s.cell_format(3), Date(DateTime, NumFmtId(164)));
        assert_eq!(s.cell_format(4), General);
        assert_eq!(s.cell_format(5), Date(Duration, NumFmtId(46)));
        assert_eq!(s.cell_format(6), General);
        assert_eq!(s.cell_format(7), General);
        assert_eq!(s.cell_format(99), General);
    }

    #[test]
    fn style_zero_can_be_a_duration() {
        let s = styles(vec![(Some(46), None)], &[]);
        assert_eq!(
            s.cell_format(0),
            CellFormat::Date(DateKind::Duration, NumFmtId(46))
        );
    }

    #[test]
    fn php_casts() {
        assert_eq!(php_int("14"), 14);
        assert_eq!(php_int(" 7x"), 7);
        assert_eq!(php_int("abc"), 0);
        assert_eq!(php_int("-3"), -3);
        assert!(php_bool("1"));
        assert!(php_bool("false")); // PHP quirk: non-empty string
        assert!(!php_bool("0"));
        assert!(!php_bool(""));
    }
}
