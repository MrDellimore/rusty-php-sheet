//! Reader behaviour on small in-memory workbooks. Each test names the OpenSpout
//! behaviour or the failure it pins down.

use super::*;
use crate::CellValue;
use crate::fuzzing::xlsx_with;

const NS: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"";

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
