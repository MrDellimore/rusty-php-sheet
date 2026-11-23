//! CSV writer with OpenSpout's semantics (`Writer\CSV\Writer`): a UTF-8 BOM by
//! default, then one `fputcsv()` line per row (no escape character). Values arrive
//! as bytes already converted the way OpenSpout converts them (that needs PHP:
//! `(string)` for floats, `DATE_ATOM` for dates), so this only handles layout.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::Result;

#[derive(Debug, Clone)]
pub struct CsvWriterOptions {
    pub delimiter: u8,
    pub enclosure: u8,
    pub bom: bool,
    pub eol: Vec<u8>,
}

impl Default for CsvWriterOptions {
    fn default() -> Self {
        CsvWriterOptions {
            delimiter: b',',
            enclosure: b'"',
            bom: true,
            eol: b"\n".to_vec(),
        }
    }
}

pub struct CsvWriter {
    out: BufWriter<File>,
    opts: CsvWriterOptions,
    line: Vec<u8>,
}

impl CsvWriter {
    pub fn create(path: &Path, opts: CsvWriterOptions) -> Result<Self> {
        let mut out = BufWriter::with_capacity(256 * 1024, File::create(path)?);
        if opts.bom {
            out.write_all(b"\xEF\xBB\xBF")?;
        }
        Ok(CsvWriter {
            out,
            opts,
            line: Vec::with_capacity(1024),
        })
    }

    pub fn add_row<F: AsRef<[u8]>>(&mut self, fields: &[F]) -> Result<()> {
        self.line.clear();
        fputcsv_line(&mut self.line, fields, &self.opts);
        self.out.write_all(&self.line)?;
        Ok(())
    }

    pub fn close(mut self) -> Result<()> {
        self.out.flush()?;
        Ok(())
    }
}

/// Port of PHP's `php_fputcsv()` without an escape character: a field is enclosed
/// if it contains the delimiter, the enclosure, `\n`, `\r`, `\t` or a space, and
/// enclosures inside it are doubled.
pub fn fputcsv_line<F: AsRef<[u8]>>(out: &mut Vec<u8>, fields: &[F], o: &CsvWriterOptions) {
    for (i, field) in fields.iter().enumerate() {
        let f = field.as_ref();
        let needs_quotes = f.iter().any(|&b| {
            b == o.delimiter || b == o.enclosure || matches!(b, b'\n' | b'\r' | b'\t' | b' ')
        });
        if needs_quotes {
            out.push(o.enclosure);
            for &b in f {
                if b == o.enclosure {
                    out.push(o.enclosure);
                }
                out.push(b);
            }
            out.push(o.enclosure);
        } else {
            out.extend_from_slice(f);
        }
        if i + 1 != fields.len() {
            out.push(o.delimiter);
        }
    }
    out.extend_from_slice(&o.eol);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(fields: &[&str]) -> String {
        let mut out = vec![];
        fputcsv_line(&mut out, fields, &CsvWriterOptions::default());
        String::from_utf8(out).unwrap()
    }

    /// Expected values produced by PHP 8.5's fputcsv($fp, $fields, ',', '"', '').
    #[test]
    fn matches_php_fputcsv() {
        assert_eq!(line(&["a", "b"]), "a,b\n");
        assert_eq!(line(&["a b", "c"]), "\"a b\",c\n");
        assert_eq!(line(&["say \"hi\"", "x,y"]), "\"say \"\"hi\"\"\",\"x,y\"\n");
        assert_eq!(line(&["", "", ""]), ",,\n");
        assert_eq!(
            line(&["tab\there", "line\nbreak", "cr\r"]),
            "\"tab\there\",\"line\nbreak\",\"cr\r\"\n"
        );
        assert_eq!(line(&["back\\slash"]), "back\\slash\n");
        let empty: [&str; 0] = [];
        assert_eq!(line(&empty), "\n");
    }
}
