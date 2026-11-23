//! Reader behaviour on small in-memory workbooks: package and workbook discovery,
//! malformed parts, and the rarer branches of the sheet, shared-string and style
//! parsers. Each test names the OpenSpout behaviour or the failure it pins down.

use super::*;
use crate::CellValue;
use crate::fuzzing::{xlsx_with, zip_of};

const NS: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"";
const REL_WS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const REL_DOC: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

fn sheet_xml(rows: &str) -> Vec<u8> {
    format!("<worksheet {NS}><sheetData>{rows}</sheetData></worksheet>").into_bytes()
}

fn book(sheet: &[u8], shared: &[u8], styles: &[u8]) -> XlsxReader {
    XlsxReader::from_bytes(xlsx_with(sheet, shared, styles)).unwrap()
}

/// All rows of sheet 0, with shared strings resolved; errors are returned in place.
fn rows_of(book: &mut XlsxReader, preserve_empty_rows: bool) -> Vec<Result<(u32, Vec<CellValue>)>> {
    let mut rows = book
        .rows(
            0,
            ReadOptions {
                preserve_empty_rows,
            },
        )
        .unwrap();
    let mut row = Row::default();
    let mut out = Vec::new();
    loop {
        match rows.next_row(&mut row) {
            Ok(true) => {
                let cells = row
                    .cells
                    .iter()
                    .map(|c| match c {
                        CellValue::SharedString(i) => {
                            CellValue::String(rows.shared_strings().get(*i).unwrap().to_string())
                        }
                        other => other.clone(),
                    })
                    .collect();
                out.push(Ok((row.number, cells)));
            }
            Ok(false) => break,
            Err(e) => {
                let fatal = !matches!(e, Error::InvalidData { .. });
                out.push(Err(e));
                if fatal {
                    break;
                }
            }
        }
    }
    // Once finished, a stream stays finished.
    assert!(!rows.next_row(&mut row).unwrap_or(false));
    out
}

fn cells(rows: &str) -> Vec<Vec<CellValue>> {
    let mut b = book(&sheet_xml(rows), b"<sst/>", b"<styleSheet/>");
    rows_of(&mut b, false)
        .into_iter()
        .map(|r| r.unwrap().1)
        .collect()
}

fn s(v: &str) -> CellValue {
    CellValue::String(v.into())
}

// ---- package ----------------------------------------------------------------

/// Patch every central-directory header of a zip: `f(header_offset, bytes)`.
fn patch_central_directory(zip: &mut [u8], f: impl Fn(usize, &mut [u8])) {
    let mut i = 0;
    while i + 4 <= zip.len() {
        if zip[i..i + 4] == [0x50, 0x4b, 0x01, 0x02] {
            f(i, zip);
        }
        i += 1;
    }
}

/// The error from opening the worksheet after patching its central-directory header.
fn open_error_after_patching_sheet(f: impl Fn(usize, &mut [u8])) -> Error {
    let mut bytes = xlsx_with(
        &sheet_xml("<row r=\"1\"><c><v>1</v></c></row>"),
        b"<sst/>",
        b"<styleSheet/>",
    );
    // Only the worksheet's header: its file name is at offset 46.
    patch_central_directory(&mut bytes, |at, zip| {
        let name_len = u16::from_le_bytes([zip[at + 28], zip[at + 29]]) as usize;
        if &zip[at + 46..at + 46 + name_len] == b"xl/worksheets/sheet1.xml" {
            f(at, zip);
        }
    });
    match XlsxReader::from_bytes(bytes)
        .unwrap()
        .rows(0, ReadOptions::default())
    {
        Err(e) => e,
        Ok(_) => panic!("the patched sheet opened"),
    }
}

#[test]
fn encrypted_parts_are_unsupported_not_garbage() {
    let err = open_error_after_patching_sheet(|at, zip| zip[at + 8] |= 1);
    assert!(err.is_unsupported(), "{err}");
    assert!(err.to_string().contains("encrypted"), "{err}");
}

#[test]
fn unknown_compression_methods_are_unsupported() {
    // 12 = bzip2: valid zip, but not something XLSX writers produce.
    let err = open_error_after_patching_sheet(|at, zip| {
        zip[at + 10] = 12;
        zip[at + 11] = 0;
    });
    assert!(
        err.is_unsupported() && err.to_string().contains("compression"),
        "{err}"
    );
}

#[test]
fn stored_and_deflated_parts_read_the_same() {
    let sheet = sheet_xml("<row r=\"1\"><c><v>7</v></c></row>");
    let parts = |deflate| {
        let base = xlsx_with(&sheet, b"<sst/>", b"<styleSheet/>");
        // Rebuild with the requested method from the same parts.
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(base)).unwrap();
        let mut owned = Vec::new();
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).unwrap();
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut data).unwrap();
            owned.push((f.name().unwrap().into_owned(), data));
        }
        let refs: Vec<(&str, &[u8])> = owned
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        zip_of(&refs, deflate)
    };
    for deflate in [false, true] {
        let mut b = XlsxReader::from_bytes(parts(deflate)).unwrap();
        assert_eq!(
            rows_of(&mut b, false)[0].as_ref().unwrap().1,
            [CellValue::Int(7)]
        );
    }
}

#[test]
fn directory_entries_are_ignored() {
    let sheet = sheet_xml("<row r=\"1\"><c><v>1</v></c></row>");
    let mut bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let opts = zip::write::SimpleFileOptions::default();
        zip.add_directory("xl/", opts).unwrap();
        let base = xlsx_with(&sheet, b"<sst/>", b"<styleSheet/>");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(base)).unwrap();
        for i in 0..archive.len() {
            zip.raw_copy_file(archive.by_index_raw(i).unwrap()).unwrap();
        }
        zip.finish().unwrap();
    }
    let mut b = XlsxReader::from_bytes(bytes).unwrap();
    assert_eq!(b.sheet_names().collect::<Vec<_>>(), ["S"]);
    assert_eq!(rows_of(&mut b, false).len(), 1);
}

// ---- workbook discovery -------------------------------------------------------

fn workbook_xml(sheets: &str, pr: &str) -> String {
    format!(
        "<workbook {NS} xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">{pr}<sheets>{sheets}</sheets></workbook>"
    )
}

fn rels(entries: &[(&str, &str, &str)]) -> String {
    let mut s = String::from(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, kind, target) in entries {
        s.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>"
        ));
    }
    s.push_str("</Relationships>");
    s
}

#[test]
fn workbook_defaults_to_the_conventional_part_without_an_office_document_rel() {
    let wb = workbook_xml("<sheet name=\"A\" sheetId=\"1\" r:id=\"r1\"/>", "");
    let wb_rels = rels(&[("r1", REL_WS, "worksheets/sheet1.xml")]);
    let sheet = sheet_xml("<row r=\"1\"><c><v>1</v></c></row>");
    let bytes = zip_of(
        &[
            ("_rels/.rels", rels(&[]).as_bytes()),
            ("xl/workbook.xml", wb.as_bytes()),
            ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
            ("xl/worksheets/sheet1.xml", &sheet),
        ],
        true,
    );
    let mut b = XlsxReader::from_bytes(bytes).unwrap();
    assert_eq!(b.sheet_names().collect::<Vec<_>>(), ["A"]);
    assert!(!b.is_1904());
    // No styles or shared-strings part at all: everything is General.
    assert_eq!(
        rows_of(&mut b, false)[0].as_ref().unwrap().1,
        [CellValue::Int(1)]
    );
}

#[test]
fn missing_workbook_part_is_reported() {
    let bytes = zip_of(
        &[(
            "_rels/.rels",
            rels(&[("r", REL_DOC, "xl/book.xml")]).as_bytes(),
        )],
        true,
    );
    match XlsxReader::from_bytes(bytes) {
        Err(Error::MissingPart(p)) => assert_eq!(p, "xl/book.xml"),
        other => panic!("expected MissingPart, got {:?}", other.err()),
    }
}

#[test]
fn workbook_at_the_package_root_finds_conventional_parts_beside_it() {
    // No workbook rels: sheets have no part and read as empty; shared strings and
    // styles are found next to the workbook by their conventional names.
    let wb = workbook_xml(
        "<sheet name=\"Orphan\" sheetId=\"1\" r:id=\"r1\"/>",
        "<workbookPr date1904=\"1\"/>",
    );
    let bytes = zip_of(
        &[
            (
                "_rels/.rels",
                rels(&[("r", REL_DOC, "/workbook.xml")]).as_bytes(),
            ),
            ("workbook.xml", wb.as_bytes()),
            ("sharedStrings.xml", b"<sst><si><t>x</t></si></sst>"),
        ],
        true,
    );
    let mut b = XlsxReader::from_bytes(bytes).unwrap();
    assert!(b.is_1904());
    assert_eq!(b.shared_strings().unwrap().get(0), Some("x"));
    assert!(rows_of(&mut b, false).is_empty());
}

#[test]
fn external_and_incomplete_relationships_are_skipped() {
    let wb = workbook_xml(
        "<sheet name=\"Kept\" sheetId=\"1\" r:id=\"r1\"/><sheet name=\"NoTarget\" sheetId=\"2\" r:id=\"r2\"/>",
        "",
    );
    let wb_rels = format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"r1\" Type=\"{REL_WS}\" Target=\"worksheets/sheet1.xml\"/>\
         <Relationship Id=\"r2\" Type=\"{REL_WS}\"/>\
         <Relationship Id=\"r3\" Type=\"x/hyperlink\" Target=\"https://example.com\" TargetMode=\"External\"/>\
         </Relationships>"
    );
    let sheet = sheet_xml("<row r=\"1\"><c><v>1</v></c></row>");
    let bytes = zip_of(
        &[
            (
                "_rels/.rels",
                rels(&[("r", REL_DOC, "xl/workbook.xml")]).as_bytes(),
            ),
            ("xl/workbook.xml", wb.as_bytes()),
            ("xl/_rels/workbook.xml.rels", wb_rels.as_bytes()),
            ("xl/worksheets/sheet1.xml", &sheet),
        ],
        true,
    );
    let mut b = XlsxReader::from_bytes(bytes).unwrap();
    assert_eq!(b.sheet_names().collect::<Vec<_>>(), ["Kept", "NoTarget"]);
    assert!(b.sheets()[1].part.is_none());
    let mut empty = b.rows(1, ReadOptions::default()).unwrap();
    assert!(!empty.next_row(&mut Row::default()).unwrap());
    assert!(matches!(
        b.rows(2, ReadOptions::default()),
        Err(Error::SheetNotFound(_))
    ));
}

#[test]
fn undecodable_attribute_escapes_are_kept_raw() {
    let wb = workbook_xml(
        "<sheet name=\"A&amp;B &bogus;\" sheetId=\"1\" r:id=\"rId1\"/>",
        "",
    );
    let bytes = zip_of(
        &[
            (
                "_rels/.rels",
                rels(&[("r", REL_DOC, "xl/workbook.xml")]).as_bytes(),
            ),
            ("xl/workbook.xml", wb.as_bytes()),
        ],
        true,
    );
    let b = XlsxReader::from_bytes(bytes).unwrap();
    assert_eq!(b.sheet_names().collect::<Vec<_>>(), ["A&amp;B &bogus;"]);
}

// ---- shared strings ------------------------------------------------------------

#[test]
fn shared_string_edge_cases() {
    let sst =
        b"<sst><si/><si><t><![CDATA[<raw> & ]]></t></si><si><t>&apos;&bogus;&#169;</t></si></sst>";
    let mut b = book(b"<worksheet/>", sst, b"<styleSheet/>");
    let table = b.shared_strings().unwrap();
    assert_eq!(table.len(), 3);
    assert!(!table.is_empty());
    assert_eq!(table.get(0), Some(""));
    assert_eq!(table.get(1), Some("<raw> &"));
    assert_eq!(table.get(2), Some("'&bogus;\u{a9}"));
    assert!(table.heap_bytes() >= "<raw> &'&bogus;©".len());
    assert!(SharedStrings::default().is_empty());
}

#[test]
fn malformed_shared_strings_are_an_xml_error() {
    let mut b = book(
        b"<worksheet/>",
        b"<sst><si><t a=\"1></t></si></sst>",
        b"<styleSheet/>",
    );
    match b.shared_strings() {
        Err(Error::Xml { part, .. }) => assert_eq!(part, "xl/sharedStrings.xml"),
        other => panic!("expected an XML error, got {:?}", other.err()),
    }
}

// ---- styles --------------------------------------------------------------------

#[test]
fn style_edge_cases() {
    let styles = br#"<styleSheet><numFmts><numFmt numFmtId="-1" formatCode="yyyy"/><numFmt numFmtId="170"/></numFmts>
        <cellXfs><xf/><xf numFmtId="+14"/><xf numFmtId="-3"/><xf numFmtId="170"/></cellXfs></styleSheet>"#;
    let rows = "<row r=\"1\"><c s=\"0\"><v>1</v></c><c s=\"1\"><v>45000</v></c><c s=\"2\"><v>2</v></c><c s=\"3\"><v>3</v></c><c s=\"99\"><v>4</v></c></row>";
    let mut b = book(&sheet_xml(rows), b"<sst/>", styles);
    let row = rows_of(&mut b, false).remove(0).unwrap().1;
    assert_eq!(row[0], CellValue::Int(1)); // no numFmtId: General
    assert!(matches!(row[1], CellValue::Date { .. })); // "+14" is 14 (PHP (int))
    assert_eq!(row[2], CellValue::Int(2)); // negative id: General
    assert_eq!(row[3], CellValue::Int(3)); // id without a code: General
    assert_eq!(row[4], CellValue::Int(4)); // unknown style index: General
}

// ---- sheet parser ----------------------------------------------------------------

#[test]
fn unusual_cell_markup() {
    let rows = concat!(
        "<row r=\"1\">",
        "<c r=\"A1\" t=\"weird\"><v>1</v></c>", // unknown type: null
        "<c r=\"B1\"><v/></c>",                 // empty <v/>: no value
        "<c r=\"C1\" t=\"str\"><v>a&amp;b</v></c>", // reference inside <v>
        "<c r=\"D1\" t=\"str\"><v><![CDATA[<x>]]></v></c>", // CDATA in <v>
        "<c r=\"E1\" t=\"inlineStr\"><is><t><![CDATA[i&]]></t></is></c>",
        "<c r=\"F1\" t=\"s\"><v>5</v></c>", // index past the table: null
        "<c r=\"G1\" foo=\"bar\"><f><![CDATA[1+1]]></f><v>2</v></c>",
        "<c r=\"H1\"><f t=\"shared\" x=\"y\">A1</f><v>1</v></c>", // shared without si: as written
        "<c r=\"I1\"><f t=\"shared\" si=\"9\"/><v>1</v></c>",     // child whose master is missing
        "&amp;<![CDATA[outside]]>",
        "</row>",
    );
    let row = cells(rows).remove(0);
    assert_eq!(row[0], CellValue::Null);
    assert_eq!(row[1], CellValue::Empty);
    assert_eq!(row[2], s("a&b"));
    assert_eq!(row[3], s("<x>"));
    assert_eq!(row[4], s("i&"));
    assert_eq!(row[5], CellValue::Null);
    let formula = |v: &CellValue| match v {
        CellValue::Formula { formula, .. } => formula.clone(),
        other => panic!("not a formula: {other:?}"),
    };
    assert_eq!(formula(&row[6]), "1+1");
    assert_eq!(formula(&row[7]), "A1");
    assert_eq!(formula(&row[8]), "");
}

#[test]
fn out_of_range_dates_are_null() {
    let styles = br#"<styleSheet><cellXfs><xf/><xf numFmtId="14"/></cellXfs></styleSheet>"#;
    let mut b = book(
        &sheet_xml(
            "<row r=\"1\"><c s=\"1\"><v>-700000</v></c><c s=\"1\"><v>1e12</v></c><c s=\"1\"><v>45000</v></c></row>",
        ),
        b"<sst/>",
        styles,
    );
    let row = rows_of(&mut b, false).remove(0).unwrap().1;
    assert_eq!(row[0], CellValue::Null);
    assert_eq!(row[1], CellValue::Null);
    assert!(matches!(row[2], CellValue::Date { .. }));
}

/// Fuzzing regression: a four-letter column used to overflow while building the
/// error message (a panic in debug builds, "column 0" in release builds).
#[test]
fn columns_beyond_xfd_are_a_recoverable_row_error() {
    let rows = "<row r=\"1\"><c r=\"ZZZZ1\"><v>1</v></c></row><row r=\"2\"><c r=\"XFE2\"/></row><row r=\"3\"><c r=\"XFD3\"><v>3</v></c></row>";
    let mut b = book(&sheet_xml(rows), b"<sst/>", b"<styleSheet/>");
    let out = rows_of(&mut b, false);
    let messages: Vec<String> = out
        .iter()
        .filter_map(|r| r.as_ref().err().map(ToString::to_string))
        .collect();
    assert_eq!(
        messages,
        [
            "invalid data in xl/worksheets/sheet1.xml: cell reference `ZZZZ1` is beyond column XFD",
            "invalid data in xl/worksheets/sheet1.xml: cell reference `XFE2` is beyond column XFD",
        ]
    );
    // The row after the bad ones still reads.
    let last = out.last().unwrap().as_ref().unwrap();
    assert_eq!(last.0, 3);
    assert_eq!(last.1.len(), 16_384);
}

#[test]
fn malformed_sheet_xml_stops_the_stream() {
    let mut b = book(
        b"<worksheet><sheetData><row r=\"1\"><c><v>1</v></c></row><row r=\"2\" a=\"></row>",
        b"<sst/>",
        b"<styleSheet/>",
    );
    let out = rows_of(&mut b, true);
    assert_eq!(out[0].as_ref().unwrap().0, 1);
    assert!(matches!(out.last().unwrap(), Err(Error::Xml { .. })));
}

/// Fuzzing regression: in preserve mode, a row numbered at or below one already
/// emitted used to move the counter back, so the next row re-emitted every gap
/// (quadratic output: a 400 KB file never finished). OpenSpout's counter only moves
/// forward: such a row becomes an empty row at the next index, its data dropped.
/// Expected values from OpenSpout 5.15 with SHOULD_PRESERVE_EMPTY_ROWS.
#[test]
fn preserve_mode_never_goes_back_like_openspout() {
    let rows = "<row r=\"1\"><c><v>1</v></c></row><row r=\"10\"><c><v>10</v></c></row>\
                <row r=\"3\"><c><v>3</v></c></row><row r=\"11\"><c><v>11</v></c></row>\
                <row r=\"13\"><c><v>13</v></c></row>";
    let mut b = book(&sheet_xml(rows), b"<sst/>", b"<styleSheet/>");
    let out: Vec<(u32, Vec<CellValue>)> = rows_of(&mut b, true)
        .into_iter()
        .map(Result::unwrap)
        .collect();
    let numbers: Vec<u32> = out.iter().map(|r| r.0).collect();
    assert_eq!(numbers, (1..=13).collect::<Vec<_>>());
    let with_data: Vec<u32> = out
        .iter()
        .filter(|r| !r.1.is_empty())
        .map(|r| r.0)
        .collect();
    assert_eq!(with_data, [1, 10, 13]);
}

/// Fuzzing regression: a corrupt `r` such as `29882414x` made preserve mode emit
/// ~30 million empty rows. Rows past Excel's last row (1,048,576) are a recoverable
/// row error, like cells past column XFD: Excel itself refuses such files.
#[test]
fn rows_beyond_the_last_excel_row_are_a_recoverable_row_error() {
    let rows = "<row r=\"1\"><c><v>1</v></c></row><row r=\"29882414x\"><c><v>2</v></c></row>\
                <row r=\"1048577\"/><row r=\"1048576\"><c><v>3</v></c></row>";
    for preserve in [false, true] {
        let mut b = book(&sheet_xml(rows), b"<sst/>", b"<styleSheet/>");
        let out = rows_of(&mut b, preserve);
        let errors: Vec<String> = out
            .iter()
            .filter_map(|r| r.as_ref().err().map(ToString::to_string))
            .collect();
        assert_eq!(
            errors,
            [
                "invalid data in xl/worksheets/sheet1.xml: row 29882414 is beyond row 1048576",
                "invalid data in xl/worksheets/sheet1.xml: row 1048577 is beyond row 1048576",
            ],
            "preserve: {preserve}"
        );
        let last = out.last().unwrap().as_ref().unwrap();
        assert_eq!(
            (last.0, last.1.clone()),
            (1_048_576, vec![CellValue::Int(3)])
        );
        let rows_read = out.iter().filter(|r| r.is_ok()).count();
        assert_eq!(rows_read, if preserve { 1_048_576 } else { 2 });
    }
}
