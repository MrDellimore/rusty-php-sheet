//! Reads every hand-crafted fixture and checks values against what OpenSpout
//! produces (after sheet-stream's trailing-'' trim), adjusted for the deliberate
//! deviations in docs/parity-notes.md.

use std::path::PathBuf;

use rusty_sheet_core::xlsx::dates::serial_to_parts;
use rusty_sheet_core::xlsx::{ReadOptions, Row, SheetRows, XlsxReader};
use rusty_sheet_core::{CellValue, DateKind};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(rel)
}

/// A compact, typed rendering of a value, close to the PHP dump format.
fn describe(rows: &SheetRows, v: &CellValue) -> String {
    match v {
        CellValue::Empty => "''".into(),
        CellValue::String(s) => format!("{s:?}"),
        CellValue::Bytes(b) => format!("bytes({b:?})"),
        CellValue::SharedString(i) => format!("{:?}", rows.shared_strings().get(*i).unwrap()),
        CellValue::Int(i) => i.to_string(),
        CellValue::Float(f) => format!("{f:?}f"),
        CellValue::Bool(b) => b.to_string(),
        CellValue::Null => "null".into(),
        CellValue::Date {
            serial,
            kind: DateKind::DateTime,
            ..
        } => {
            let p = serial_to_parts(*serial);
            format!(
                "date({:04}-{:02}-{:02} {:02}:{:02}:{:02})",
                p.year, p.month, p.day, p.hour, p.minute, p.second
            )
        }
        CellValue::Date {
            serial,
            kind: DateKind::Duration,
            ..
        } => format!("duration({serial})"),
        CellValue::IsoDate(s) => format!("iso({s})"),
        CellValue::Formula { formula, cached } => match cached {
            Some(c) => format!("={formula} [{}]", describe(rows, c)),
            None => format!("={formula} [-]"),
        },
    }
}

/// All rows of one sheet as (row number, rendered cells).
fn read(rel: &str, sheet: usize, opts: ReadOptions) -> Vec<(u32, Vec<String>)> {
    let mut book = XlsxReader::open(&fixture(rel)).expect("open");
    let mut rows = book.rows(sheet, opts).expect("rows");
    let mut row = Row::default();
    let mut out = Vec::new();
    while rows.next_row(&mut row).expect("row") {
        let cells = row.cells.iter().map(|c| describe(&rows, c)).collect();
        out.push((row.number, cells));
    }
    out
}

/// Column B of each row (rows are `label, value`), or "<none>" if trimmed away.
fn column_b(rows: &[(u32, Vec<String>)]) -> Vec<String> {
    rows.iter()
        .map(|(_, cells)| cells.get(1).cloned().unwrap_or_else(|| "<none>".into()))
        .collect()
}

#[test]
fn cell_types() {
    let rows = read("edge/cell-types.xlsx", 0, ReadOptions::default());
    assert_eq!(
        column_b(&rows),
        [
            "\"hello shared\"",
            "\"hello inline\"",
            "1500",
            "0.075f",
            "-42",
            "0.0015f",
            "1000",
            "1e20f",
            "3",
            "7",
            "true",
            "false",
            "null",
            "null",
            "\"plain str\"",
            "iso(2025-06-15T13:45:00)",
            "<none>", // empty inline string is trimmed
            "''",
            "null", // unparseable number: OpenSpout gives 0.0 (deliberate deviation)
            "''",
        ]
    );
    // A styled blank mid-row is '', followed by the real value.
    assert_eq!(rows[17].1[1..], ["''", "1"]);
    assert_eq!(rows[19].1[1..], ["''", "2"]);
}

#[test]
fn layout_skips_empty_rows_and_trims() {
    let rows = read("edge/layout.xlsx", 0, ReadOptions::default());
    let expected: Vec<(u32, Vec<&str>)> = vec![
        (1, vec!["\"a\"", "''", "\"c\"", "''", "''", "\"f\""]),
        (2, vec!["\"cells without r\"", "1", "2"]),
        (6, vec!["\"trailing blanks\""]),
        (7, vec!["\"row without r\""]),
        (10, vec!["\"after gap of empty rows\"", "10"]),
        (
            11,
            vec![
                "\"row attributes + inline rich text\"",
                "''",
                "\"rich inline\"",
            ],
        ),
    ];
    let got: Vec<(u32, Vec<&str>)> = rows
        .iter()
        .map(|(n, c)| (*n, c.iter().map(String::as_str).collect()))
        .collect();
    assert_eq!(got, expected);
}

#[test]
fn layout_preserves_empty_rows() {
    let rows = read(
        "edge/layout.xlsx",
        0,
        ReadOptions {
            preserve_empty_rows: true,
        },
    );
    let numbers: Vec<u32> = rows.iter().map(|(n, _)| *n).collect();
    assert_eq!(numbers, (1..=11).collect::<Vec<_>>());
    let empty: Vec<u32> = rows
        .iter()
        .filter(|(_, c)| c.is_empty())
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(empty, [3, 4, 5, 8, 9]);
}

#[test]
fn shared_string_decoding() {
    let rows = read("edge/layout.xlsx", 1, ReadOptions::default());
    assert_eq!(
        column_b(&rows),
        [
            "\"plain\"",
            "\"  padded  \"",
            "\"bold and plain\"",
            "\"東京\"",
            "\"line1_x000D_\\nline2\"",
            "\"Tom & Jerry <3> \\\"q\\\" ©\"",
            "<none>",
            "\"café 🚀\"",
            "\"_x00zz_ and _X000D_ and __x0041__\"",
            "\"tab\\there\"",
        ]
    );
}

#[test]
fn date_detection_matches_openspout() {
    let rows = read("edge/dates.xlsx", 0, ReadOptions::default());
    // Row 1 is the header; one row per format follows.
    let kind = |cells: &Vec<String>| {
        let b = &cells[1];
        if b.starts_with("date(") {
            "date"
        } else if b.starts_with("duration(") {
            "duration"
        } else {
            "number"
        }
    };
    let kinds: Vec<&str> = rows[1..].iter().map(|(_, c)| kind(c)).collect();
    let mut expected = vec!["date"; 28];
    for number_row in [2, 15, 21, 22, 23, 27, 29] {
        expected[number_row - 2] = "number";
    }
    for duration_row in [13, 20] {
        expected[duration_row - 2] = "duration";
    }
    assert_eq!(kinds, expected);

    // Built-in 14: OpenSpout's datetime for each serial.
    assert_eq!(
        rows[2].1[1..],
        [
            "date(2025-06-15 00:00:00)",
            "date(2025-06-15 12:00:00)",
            "date(1899-12-30 18:00:00)",
            "date(1899-12-31 00:00:00)",
            "date(1900-02-27 00:00:00)",
            "date(1900-02-28 00:00:00)",
            "date(1900-03-01 00:00:00)",
            "date(1899-12-30 00:00:00)",
            "date(2025-06-15 23:59:59)",
        ]
    );
    // General keeps int vs float.
    assert_eq!(
        rows[1].1[1..],
        [
            "45823",
            "45823.5f",
            "0.75f",
            "1",
            "59",
            "60",
            "61",
            "0",
            "45823.999988426f"
        ]
    );
}

#[test]
fn dates_1904_are_normalised() {
    let rows = read("edge/dates-1904.xlsx", 0, ReadOptions::default());
    let b: Vec<&str> = rows.iter().map(|(_, c)| c[1].as_str()).collect();
    assert_eq!(
        b,
        [
            "date(1904-01-01 00:00:00)",
            "date(1904-01-02 00:00:00)",
            "date(2025-06-15 00:00:00)",
            "date(2025-06-15 06:00:00)",
        ]
    );
    // General-formatted serials are untouched numbers.
    assert_eq!(rows[0].1[3], "0");
    assert_eq!(rows[3].1[3], "44361.25f");
}

#[test]
fn formulas_with_shared_expansion() {
    let rows = read("edge/formulas.xlsx", 0, ReadOptions::default());
    let d: Vec<&str> = rows.iter().map(|(_, c)| c[3].as_str()).collect();
    assert_eq!(
        d,
        [
            "=B1+C1 [30]",
            "=\"a\"&\"b\" [\"ab\"]",
            "=B3>C3 [false]",
            "=B4/C4 [null]", // error result: cached, but null (OpenSpout: null)
            "=B5*2 [10]",
            "=B6*2 [12]", // shared child: OpenSpout gives "=" (deliberate deviation)
            "=B7*2 [14]",
            "=B8*3 [-]",
            "=B9 [date(2025-06-15 00:00:00)]",
            "=B10/C10 [3.3333333333333335f]",
            "=SUM(B11:C11) [9]",
            "=B12*C12/1 [6]",
        ]
    );
}

#[test]
fn multi_sheet_order_and_paths() {
    let mut book = XlsxReader::open(&fixture("edge/multi-sheet.xlsx")).unwrap();
    let names: Vec<&str> = book.sheet_names().collect();
    assert_eq!(names, ["Zeta", "Alpha & Ω", "Empty", "Last"]);
    assert_eq!(book.sheet_index("Last"), Some(3));
    drop(
        book.rows(9, ReadOptions::default())
            .err()
            .expect("no such sheet"),
    );

    for (i, name) in ["Zeta", "Alpha & Ω", "Empty", "Last"].iter().enumerate() {
        let rows = read("edge/multi-sheet.xlsx", i, ReadOptions::default());
        if *name == "Empty" {
            assert!(rows.is_empty());
        } else {
            assert_eq!(
                rows,
                [(
                    1,
                    vec![format!("{:?}", format!("sheet {name}")), "1".into()]
                )]
            );
        }
    }
}

#[test]
fn every_fixture_reads_without_error() {
    let dirs = ["edge", "laravel-excel"];
    let mut files = 0;
    for dir in dirs {
        for entry in std::fs::read_dir(fixture(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("xlsx") {
                continue;
            }
            let mut book =
                XlsxReader::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            for i in 0..book.sheets().len() {
                let mut rows = book.rows(i, ReadOptions::default()).unwrap();
                let mut row = Row::default();
                while rows
                    .next_row(&mut row)
                    .unwrap_or_else(|e| panic!("{} sheet {i}: {e}", path.display()))
                {
                }
            }
            files += 1;
        }
    }
    assert!(files >= 30, "only {files} fixtures found");
}

#[test]
fn non_xlsx_inputs_fail_cleanly() {
    assert!(XlsxReader::open(&fixture("laravel-excel/import-external-reference.xls")).is_err());
    assert!(XlsxReader::open(&fixture("laravel-excel/csv-with-comma.csv")).is_err());
    assert!(XlsxReader::open(&fixture("laravel-excel/unknown-reader-type.zip")).is_err());
    assert!(XlsxReader::open(&fixture("does-not-exist.xlsx")).is_err());
}
