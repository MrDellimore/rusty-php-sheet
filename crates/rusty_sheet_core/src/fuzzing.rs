//! Entry points for `cargo fuzz` (the `fuzzing` feature; never part of the extension).
//!
//! Each function feeds arbitrary bytes to one parser through its public path and
//! drains it. Errors are expected and ignored; only a panic, a hang or unbounded
//! memory counts as a failure. Every input the fuzzer finds becomes a regression
//! test next to the code it broke (see `xlsx/read_paths.rs`).

use std::io::{Cursor, Write};

use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::csv::{CsvOptions, CsvReader};
use crate::xlsx::{ReadOptions, Row, XlsxReader};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;

const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;

const ONE_ROW_SHEET: &[u8] = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s" s="1"><v>0</v></c><c r="B1" s="2"><v>45000.5</v></c></row></sheetData></worksheet>"#;

/// A zip of `(name, bytes)` parts, stored or deflated.
pub fn zip_of(parts: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let method = if deflate {
        zip::CompressionMethod::Deflated
    } else {
        zip::CompressionMethod::Stored
    };
    let opts = SimpleFileOptions::default().compression_method(method);
    for (name, data) in parts {
        zip.start_file(*name, opts).expect("in-memory zip");
        zip.write_all(data).expect("in-memory zip");
    }
    zip.finish().expect("in-memory zip").into_inner()
}

/// A minimal XLSX whose sheet, shared-strings and styles parts are given.
pub fn xlsx_with(sheet: &[u8], shared_strings: &[u8], styles: &[u8]) -> Vec<u8> {
    zip_of(
        &[
            ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
            ("_rels/.rels", ROOT_RELS.as_bytes()),
            ("xl/workbook.xml", WORKBOOK.as_bytes()),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.as_bytes()),
            ("xl/worksheets/sheet1.xml", sheet),
            ("xl/sharedStrings.xml", shared_strings),
            ("xl/styles.xml", styles),
        ],
        false,
    )
}

/// Read every row of every sheet, in both empty-row modes. Returns the rows read.
pub fn read_xlsx(bytes: &[u8]) -> usize {
    let mut total = 0;
    for preserve_empty_rows in [false, true] {
        let Ok(mut book) = XlsxReader::from_bytes(bytes.to_vec()) else {
            return total;
        };
        for i in 0..book.sheets().len() {
            let Ok(mut rows) = book.rows(
                i,
                ReadOptions {
                    preserve_empty_rows,
                },
            ) else {
                continue;
            };
            let mut row = Row::default();
            // A recoverable error (e.g. a column past XFD) leaves the parser past the
            // bad row; the PHP layer may carry on, so the fuzzer does too, a bounded
            // number of times.
            let mut errors = 0;
            loop {
                match rows.next_row(&mut row) {
                    Ok(true) => {
                        total += 1;
                        for cell in &row.cells {
                            std::hint::black_box(rows.shared_strings().len());
                            std::hint::black_box(cell);
                        }
                    }
                    Ok(false) => break,
                    Err(_) => {
                        errors += 1;
                        if errors > 64 {
                            break;
                        }
                    }
                }
            }
        }
    }
    total
}

/// The sheet parser: `data` is the worksheet XML.
pub fn sheet(data: &[u8]) {
    let _ = read_xlsx(&xlsx_with(data, b"<sst/>", b"<styleSheet/>"));
}

/// The shared-strings parser: `data` is `sharedStrings.xml`.
pub fn shared_strings(data: &[u8]) {
    let _ = read_xlsx(&xlsx_with(ONE_ROW_SHEET, data, b"<styleSheet/>"));
}

/// The styles parser (and date-format detection): `data` is `styles.xml`.
pub fn styles(data: &[u8]) {
    let _ = read_xlsx(&xlsx_with(
        ONE_ROW_SHEET,
        b"<sst><si><t>x</t></si></sst>",
        data,
    ));
}

/// The CSV parser. The first byte picks the options, the rest is the file.
pub fn csv(data: &[u8]) {
    let Some((&selector, input)) = data.split_first() else {
        return;
    };
    let opts = CsvOptions {
        delimiter: b",;\t|"[usize::from(selector & 3)],
        enclosure: if selector & 4 == 0 { b'"' } else { b'\'' },
        escape: if selector & 8 == 0 { None } else { Some(b'\\') },
        preserve_empty_rows: selector & 16 != 0,
    };
    let Ok(mut reader) = CsvReader::from_bytes(input.to_vec(), opts) else {
        return;
    };
    let mut row = Row::default();
    while let Ok(true) = reader.next_row(&mut row) {
        std::hint::black_box(&row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The harness itself must work: a well-formed input reads without panicking.
    #[test]
    fn entry_points_accept_valid_and_garbage_input() {
        // The harness must reach the parsers: a valid sheet yields its row in both modes.
        assert_eq!(
            read_xlsx(&xlsx_with(
                ONE_ROW_SHEET,
                b"<sst><si><t>x</t></si></sst>",
                b"<styleSheet/>"
            )),
            2
        );
        sheet(ONE_ROW_SHEET);
        sheet(b"<worksheet><sheetData><row><c r=\"ZZZZ1\"/></row>");
        shared_strings(b"<sst><si><r><t>a</t></r><t>b</t></si></sst>");
        styles(b"<styleSheet><numFmts><numFmt numFmtId=\"164\" formatCode=\"yyyy\"/></numFmts><cellXfs><xf numFmtId=\"164\"/></cellXfs></styleSheet>");
        csv(b"\x00a,b\n\"c");
        csv(b"");
        assert_eq!(read_xlsx(b"not a zip"), 0);
    }
}
