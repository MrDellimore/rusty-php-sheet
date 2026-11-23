//! Small hand-crafted XLSX files, one per area of reader behaviour.
//!
//! Every file is deterministic and committed under `tests/fixtures/edge/`.
//! OpenSpout's output for these files is the parity reference (Phase 2), so the
//! fixtures cover inputs; they don't assert outputs. Column A of most rows holds a
//! label describing what the row exercises.

use std::io;
use std::path::Path;

use crate::xlsx::{self, BookSpec, Package, SheetEntry};

struct Fixture {
    file: &'static str,
    description: &'static str,
    spec: BookSpec,
    /// Raw `<si>` inner XML, or empty when the workbook has no shared strings.
    sst: Vec<String>,
    /// Complete worksheet XML, one per `spec.sheets` entry.
    sheets: Vec<String>,
}

/// Inline-string label cell.
fn label(cell: &str, text: &str) -> String {
    format!(
        "<c r=\"{cell}\" t=\"inlineStr\"><is><t>{}</t></is></c>",
        xlsx::xml_escape(text)
    )
}

/// A row with a label in column A followed by raw cell XML.
fn labelled_row(r: usize, text: &str, cells: &str) -> String {
    format!(
        "<row r=\"{r}\">{}{cells}</row>",
        label(&format!("A{r}"), text)
    )
}

fn cell_types() -> Fixture {
    let rows: &[(&str, &str)] = &[
        ("shared string", "<c r=\"B{r}\" t=\"s\"><v>0</v></c>"),
        (
            "inline string",
            "<c r=\"B{r}\" t=\"inlineStr\"><is><t>hello inline</t></is></c>",
        ),
        ("int", "<c r=\"B{r}\"><v>1500</v></c>"),
        ("float", "<c r=\"B{r}\"><v>0.075</v></c>"),
        ("negative int", "<c r=\"B{r}\"><v>-42</v></c>"),
        ("exponent notation", "<c r=\"B{r}\"><v>1.5E-3</v></c>"),
        ("integral exponent", "<c r=\"B{r}\"><v>1E+3</v></c>"),
        ("beyond i64", "<c r=\"B{r}\"><v>1E+20</v></c>"),
        ("integral written as 3.0", "<c r=\"B{r}\"><v>3.0</v></c>"),
        ("explicit t=n", "<c r=\"B{r}\" t=\"n\"><v>7</v></c>"),
        ("bool true", "<c r=\"B{r}\" t=\"b\"><v>1</v></c>"),
        ("bool false", "<c r=\"B{r}\" t=\"b\"><v>0</v></c>"),
        ("error #N/A", "<c r=\"B{r}\" t=\"e\"><v>#N/A</v></c>"),
        ("error #DIV/0!", "<c r=\"B{r}\" t=\"e\"><v>#DIV/0!</v></c>"),
        (
            "t=str without formula",
            "<c r=\"B{r}\" t=\"str\"><v>plain str</v></c>",
        ),
        (
            "ISO date t=d",
            "<c r=\"B{r}\" t=\"d\"><v>2025-06-15T13:45:00</v></c>",
        ),
        (
            "empty inline string",
            "<c r=\"B{r}\" t=\"inlineStr\"><is><t/></is></c>",
        ),
        (
            "styled blank, value in C",
            "<c r=\"B{r}\" s=\"0\"/><c r=\"C{r}\"><v>1</v></c>",
        ),
        ("unparseable number", "<c r=\"B{r}\"><v>abc</v></c>"),
        (
            "number with no v",
            "<c r=\"B{r}\"></c><c r=\"C{r}\"><v>2</v></c>",
        ),
    ];
    let mut data = String::new();
    for (i, (text, cell)) in rows.iter().enumerate() {
        let r = i + 1;
        data.push_str(&labelled_row(r, text, &cell.replace("{r}", &r.to_string())));
    }
    Fixture {
        file: "cell-types.xlsx",
        description: "One value per row covering every cell type (`s`, `inlineStr`, `n`, `b`, `e`, `str`, `d`), \
                      int-vs-float edge cases, blanks and an unparseable number.",
        spec: BookSpec {
            sheets: vec![SheetEntry::standard(1, "Types")],
            has_shared_strings: true,
            num_fmts: vec![],
            cell_xfs: vec![],
            date1904: false,
        },
        sst: vec!["<t>hello shared</t>".into()],
        sheets: vec![xlsx::worksheet_xml(None, &data)],
    }
}

fn layout() -> Fixture {
    // Sheet 1: row/column positioning. The dimension is deliberately wider than the
    // data: OpenSpout 5 pads rows to it with '' and sheet-stream trims them.
    let mut rows = String::new();
    rows.push_str(&format!(
        "<row r=\"1\">{}{}{}</row>",
        label("A1", "a"),
        label("C1", "c"),
        label("F1", "f")
    ));
    rows.push_str(
        "<row r=\"2\"><c t=\"inlineStr\"><is><t>cells without r</t></is></c><c><v>1</v></c><c><v>2</v></c></row>",
    );
    // Row 3 is absent entirely.
    rows.push_str("<row r=\"4\"/>");
    rows.push_str(
        "<row r=\"5\"><c r=\"A5\" t=\"inlineStr\"><is><t></t></is></c><c r=\"B5\" s=\"0\"/></row>",
    );
    rows.push_str(&format!(
        "<row r=\"6\">{}<c r=\"B6\" t=\"inlineStr\"><is><t></t></is></c><c r=\"C6\" s=\"0\"/></row>",
        label("A6", "trailing blanks")
    ));
    rows.push_str(&format!("<row>{}</row>", label("A7", "row without r")));
    rows.push_str(&format!(
        "<row r=\"10\">{}<c r=\"B10\"><v>10</v></c></row>",
        label("A10", "after gap of empty rows")
    ));
    rows.push_str(&format!(
        "<row r=\"11\" spans=\"1:3\" ht=\"30\" customHeight=\"1\">{}<c r=\"C11\" t=\"inlineStr\"><is><r><t>rich </t></r><r><rPr><b/></rPr><t>inline</t></r></is></c></row>",
        label("A11", "row attributes + inline rich text")
    ));
    let layout_sheet = xlsx::worksheet_xml(Some("A1:Z20"), &rows);

    // Sheet 2: shared-string decoding.
    let sst = vec![
        "<t>plain</t>".to_string(),
        "<t xml:space=\"preserve\">  padded  </t>".into(),
        "<r><rPr><b/></rPr><t>bold</t></r><r><t xml:space=\"preserve\"> and plain</t></r>".into(),
        "<t>東京</t><rPh sb=\"0\" eb=\"2\"><t>トウキョウ</t></rPh><phoneticPr fontId=\"1\"/>"
            .into(),
        "<t>line1_x000D_\nline2</t>".into(),
        "<t>Tom &amp; Jerry &lt;3&gt; &quot;q&quot; &#169;</t>".into(),
        "<t/>".into(),
        "<t>café 🚀</t>".into(),
        "<t>_x00zz_ and _X000D_ and __x0041__</t>".into(),
        "<t>tab\there</t>".into(),
    ];
    let labels = [
        "plain",
        "xml:space preserve",
        "rich text runs",
        "phonetic run excluded",
        "_x000D_ escape + newline",
        "entities",
        "empty string",
        "unicode",
        "not-quite escapes",
        "tab",
    ];
    let mut strings = String::new();
    for (i, text) in labels.iter().enumerate() {
        let r = i + 1;
        strings.push_str(&labelled_row(
            r,
            text,
            &format!("<c r=\"B{r}\" t=\"s\"><v>{i}</v></c>"),
        ));
    }

    Fixture {
        file: "layout.xlsx",
        description: "Sheet `Layout`: column gaps, cells/rows without `r`, missing and empty rows, \
                      trailing blanks, a `<dimension>` wider than the data. Sheet `Strings`: shared-string \
                      decoding (rich text, phonetic runs, `xml:space`, `_xHHHH_` escapes, entities, unicode).",
        spec: BookSpec {
            sheets: vec![
                SheetEntry::standard(1, "Layout"),
                SheetEntry::standard(2, "Strings"),
            ],
            has_shared_strings: true,
            num_fmts: vec![],
            cell_xfs: vec![],
            date1904: false,
        },
        sst,
        sheets: vec![layout_sheet, xlsx::worksheet_xml(None, &strings)],
    }
}

/// `(numFmtId, custom code or None for built-in, label)`
const DATE_FORMATS: &[(u32, Option<&str>, &str)] = &[
    (0, None, "General (not a date)"),
    (14, None, "built-in 14 m/d/yyyy"),
    (15, None, "built-in 15 d-mmm-yy"),
    (16, None, "built-in 16 d-mmm"),
    (17, None, "built-in 17 mmm-yy"),
    (18, None, "built-in 18 h:mm AM/PM"),
    (19, None, "built-in 19 h:mm:ss AM/PM"),
    (20, None, "built-in 20 h:mm"),
    (21, None, "built-in 21 h:mm:ss"),
    (22, None, "built-in 22 m/d/yyyy h:mm"),
    (45, None, "built-in 45 mm:ss"),
    (46, None, "built-in 46 [h]:mm:ss"),
    (47, None, "built-in 47 mm:ss.0"),
    (4, None, "built-in 4 #,##0.00 (not a date)"),
    (164, Some("yyyy-mm-dd"), "custom yyyy-mm-dd"),
    (165, Some("dd/mm/yyyy hh:mm"), "custom dd/mm/yyyy hh:mm"),
    (166, Some("[$-409]mmmm d, yyyy;@"), "custom locale prefix"),
    (167, Some("\"Due \"yyyy"), "custom quoted literal + yyyy"),
    (168, Some("[h]:mm:ss"), "custom elapsed hours"),
    (169, Some("0.00"), "custom 0.00 (not a date)"),
    (170, Some("\"y\"0"), "custom quoted y (not a date)"),
    (171, Some("[Red]0.00"), "custom [Red] (not a date)"),
    (172, Some("mm:ss"), "custom mm:ss"),
    (173, Some("d-mmm"), "custom d-mmm"),
    (174, Some("h:mm AM/PM"), "custom h:mm AM/PM"),
    (175, Some("0.00E+00"), "custom scientific (not a date)"),
    (
        176,
        Some("yyyy-mm-dd hh:mm:ss"),
        "custom yyyy-mm-dd hh:mm:ss",
    ),
    (
        177,
        Some("\\d\\a\\y 0"),
        "custom escaped letters (not a date)",
    ),
];

/// Serial values placed in columns B.. of each format row.
const DATE_VALUES: &[(&str, &str)] = &[
    ("45823", "2025-06-15"),
    ("45823.5", "2025-06-15 12:00"),
    ("0.75", "18:00 time only"),
    ("1", "1900-01-01"),
    ("59", "1900-02-28"),
    ("60", "fictitious 1900-02-29"),
    ("61", "1900-03-01"),
    ("0", "zero"),
    ("45823.999988426", "2025-06-15 23:59:59"),
];

fn dates() -> Fixture {
    let mut rows = String::new();
    // Header row describing the value columns.
    rows.push_str("<row r=\"1\">");
    rows.push_str(&label("A1", "format \\ value"));
    for (c, (_, desc)) in DATE_VALUES.iter().enumerate() {
        rows.push_str(&label(&format!("{}1", xlsx::col_letters(c + 1)), desc));
    }
    rows.push_str("</row>");

    let mut cell_xfs = vec![0u32];
    let mut num_fmts = vec![];
    for (i, (id, code, text)) in DATE_FORMATS.iter().enumerate() {
        let r = i + 2;
        let style = cell_xfs.len();
        cell_xfs.push(*id);
        if let Some(code) = code {
            num_fmts.push((*id, code.to_string()));
        }
        let mut cells = String::new();
        for (c, (v, _)) in DATE_VALUES.iter().enumerate() {
            cells.push_str(&format!(
                "<c r=\"{}{r}\" s=\"{style}\"><v>{v}</v></c>",
                xlsx::col_letters(c + 1)
            ));
        }
        rows.push_str(&labelled_row(r, text, &cells));
    }

    Fixture {
        file: "dates.xlsx",
        description: "Rows = number formats (built-in date IDs 14–22 and 45–47, custom date/time codes, and \
                      non-date look-alikes such as `[Red]0.00` and `\"y\"0`). Columns = serial values \
                      including the 1900 leap-year bug around serial 60 and time-only fractions.",
        spec: BookSpec {
            sheets: vec![SheetEntry::standard(1, "Dates")],
            has_shared_strings: false,
            num_fmts,
            cell_xfs,
            date1904: false,
        },
        sst: vec![],
        sheets: vec![xlsx::worksheet_xml(None, &rows)],
    }
}

fn dates_1904() -> Fixture {
    let values = [
        ("0", "1904-01-01"),
        ("1", "1904-01-02"),
        ("44361", "2025-06-15"),
        ("44361.25", "2025-06-15 06:00"),
    ];
    let mut rows = String::new();
    for (i, (v, desc)) in values.iter().enumerate() {
        let r = i + 1;
        rows.push_str(&labelled_row(
            r,
            desc,
            &format!("<c r=\"B{r}\" s=\"1\"><v>{v}</v></c><c r=\"C{r}\" s=\"2\"><v>{v}</v></c><c r=\"D{r}\"><v>{v}</v></c>"),
        ));
    }
    Fixture {
        file: "dates-1904.xlsx",
        description: "Workbook with `date1904=\"1\"`. Same serials formatted as date (B), datetime (C) and \
                      General (D).",
        spec: BookSpec {
            sheets: vec![SheetEntry::standard(1, "Mac Dates")],
            has_shared_strings: false,
            num_fmts: vec![],
            cell_xfs: vec![0, 14, 22],
            date1904: true,
        },
        sst: vec![],
        sheets: vec![xlsx::worksheet_xml(None, &rows)],
    }
}

fn formulas() -> Fixture {
    // Columns: A label, B and C operands, D formula.
    let rows: &[(&str, &str, &str, &str)] = &[
        (
            "numeric result",
            "10",
            "20",
            "<c r=\"D{r}\"><f>B{r}+C{r}</f><v>30</v></c>",
        ),
        (
            "string result",
            "1",
            "2",
            "<c r=\"D{r}\" t=\"str\"><f>\"a\"&amp;\"b\"</f><v>ab</v></c>",
        ),
        (
            "bool result",
            "1",
            "2",
            "<c r=\"D{r}\" t=\"b\"><f>B{r}&gt;C{r}</f><v>0</v></c>",
        ),
        (
            "error result",
            "1",
            "0",
            "<c r=\"D{r}\" t=\"e\"><f>B{r}/C{r}</f><v>#DIV/0!</v></c>",
        ),
        (
            "shared formula master",
            "5",
            "0",
            "<c r=\"D{r}\"><f t=\"shared\" ref=\"D5:D7\" si=\"0\">B5*2</f><v>10</v></c>",
        ),
        (
            "shared formula child",
            "6",
            "0",
            "<c r=\"D{r}\"><f t=\"shared\" si=\"0\"/><v>12</v></c>",
        ),
        (
            "shared formula child",
            "7",
            "0",
            "<c r=\"D{r}\"><f t=\"shared\" si=\"0\"/><v>14</v></c>",
        ),
        (
            "no cached value",
            "3",
            "0",
            "<c r=\"D{r}\"><f>B{r}*3</f></c>",
        ),
        (
            "date-formatted result",
            "45823",
            "0",
            "<c r=\"D{r}\" s=\"1\"><f>B{r}</f><v>45823</v></c>",
        ),
        (
            "float result",
            "10",
            "3",
            "<c r=\"D{r}\"><f>B{r}/C{r}</f><v>3.3333333333333335</v></c>",
        ),
        (
            "array formula",
            "4",
            "5",
            "<c r=\"D{r}\"><f t=\"array\" ref=\"D{r}\">SUM(B{r}:C{r})</f><v>9</v></c>",
        ),
        (
            "integral float result",
            "3",
            "2",
            "<c r=\"D{r}\"><f>B{r}*C{r}/1</f><v>6.0</v></c>",
        ),
    ];
    let mut data = String::new();
    for (i, (text, b, c, d)) in rows.iter().enumerate() {
        let r = i + 1;
        let cells = format!(
            "<c r=\"B{r}\"><v>{b}</v></c><c r=\"C{r}\"><v>{c}</v></c>{}",
            d.replace("{r}", &r.to_string())
        );
        data.push_str(&labelled_row(r, text, &cells));
    }
    Fixture {
        file: "formulas.xlsx",
        description: "Formula cells with numeric, string, bool, error, date-formatted and missing cached \
                      values, plus shared (`t=\"shared\"`, children have no formula text) and array formulas.",
        spec: BookSpec {
            sheets: vec![SheetEntry::standard(1, "Formulas")],
            has_shared_strings: false,
            num_fmts: vec![],
            cell_xfs: vec![0, 14],
            date1904: false,
        },
        sst: vec![],
        sheets: vec![xlsx::worksheet_xml(None, &data)],
    }
}

fn multi_sheet() -> Fixture {
    let marker = |name: &str| {
        xlsx::worksheet_xml(
            None,
            &format!(
                "<row r=\"1\">{}<c r=\"B1\"><v>1</v></c></row>",
                label("A1", &format!("sheet {name}"))
            ),
        )
    };
    let sheets = vec![
        // Workbook order differs from part numbering and rel id order.
        SheetEntry {
            name: "Zeta".into(),
            part: "xl/worksheets/sheet3.xml".into(),
            rel_target: "worksheets/sheet3.xml".into(),
            rel_id: "rId7".into(),
        },
        // Absolute relationship target.
        SheetEntry {
            name: "Alpha & Ω".into(),
            part: "xl/worksheets/sheet1.xml".into(),
            rel_target: "/xl/worksheets/sheet1.xml".into(),
            rel_id: "rId2".into(),
        },
        SheetEntry {
            name: "Empty".into(),
            part: "xl/worksheets/sheet2.xml".into(),
            rel_target: "worksheets/sheet2.xml".into(),
            rel_id: "rId3".into(),
        },
        // Non-standard part location.
        SheetEntry {
            name: "Last".into(),
            part: "xl/worksheets/data/custom-name.xml".into(),
            rel_target: "worksheets/data/custom-name.xml".into(),
            rel_id: "rId1".into(),
        },
    ];
    let empty = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <worksheet xmlns=\"{}\"><dimension ref=\"A1\"/><sheetData/></worksheet>",
        xlsx::NS_MAIN
    );
    Fixture {
        file: "multi-sheet.xlsx",
        description: "Four sheets whose workbook order differs from part numbers and relationship ids; \
                      an absolute relationship target, a non-standard part path, a name needing XML \
                      escaping, a self-closing empty `<sheetData/>`, and no shared-strings part.",
        spec: BookSpec {
            sheets,
            has_shared_strings: false,
            num_fmts: vec![],
            cell_xfs: vec![],
            date1904: false,
        },
        sst: vec![],
        sheets: vec![marker("Zeta"), marker("Alpha & Ω"), empty, marker("Last")],
    }
}

/// Format codes for rendering (`dates => 'formatted'` / OpenSpout SHOULD_FORMAT_DATES).
const RENDER_FORMATS: &[&str] = &[
    "m/d/yyyy",
    "d-mmm-yy",
    "h:mm AM/PM",
    "h:mm:ss AM/PM",
    "hh:mm AM/PM",
    "m/d/yyyy h:mm",
    "mm:ss",
    "mm:ss.0",
    "yyyy-mm-dd hh:mm:ss",
    "dddd, mmmm d, yyyy",
    "ddd mmm d yy",
    "mmmmm yyyy",
    "[$-409]mmmm d, yyyy;@",
    "[$-409]mmm d;@",
    "yyyy-mm-dd;@",
    "\"Day \"d\" of \"mmmm",
    "yyyy-mm-dd\\Thh:mm",
    "yyyy\\-mm",
    "dd/mm/yyyy",
    "d/m/yy h:mm",
    "e",
    "yy",
    "h \"o'clock\"",
    "hh:mm \"AM/PM\"",
    "[Red]yyyy-mm-dd",
    "[Blue]d mmm",
    "dd.mm.yyyy hh:mm:ss",
    "yyyy\"年\"m\"月\"d\"日\"",
    "mmm-yy",
    "m/d",
    "h:mm",
    "h:mm:ss",
    "[h]:mm:ss",
    "[hh]:mm",
    "[h]",
    "[mm]:ss",
    "[m]",
    "[ss]",
    "[s]",
    "[h]:mm:ss.0",
];

const RENDER_VALUES: &[&str] = &[
    "45823.5",         // Sunday 2025-06-15 12:00
    "45658",           // Wednesday 2025-01-01 00:00
    "45700.208333333", // 2025-02-12 05:00
    "0.75",            // 18:00 (time only)
    "1.5",             // 1899-12-31 12:00, or a 36-hour duration
    "45823.999988426", // 23:59:59
    "-0.25",           // negative: before the epoch, or a negative duration
];

fn date_formats() -> Fixture {
    let mut cell_xfs = vec![0u32];
    let mut num_fmts = vec![];
    let mut rows = String::new();
    for (i, code) in RENDER_FORMATS.iter().enumerate() {
        let r = i + 1;
        let id = 164 + i as u32;
        num_fmts.push((id, code.to_string()));
        let style = cell_xfs.len();
        cell_xfs.push(id);
        let mut cells = String::new();
        for (c, v) in RENDER_VALUES.iter().enumerate() {
            cells.push_str(&format!(
                "<c r=\"{}{r}\" s=\"{style}\"><v>{v}</v></c>",
                xlsx::col_letters(c + 1)
            ));
        }
        rows.push_str(&labelled_row(r, code, &cells));
    }
    Fixture {
        file: "date-formats.xlsx",
        description: "Rendering of date, time and duration format codes (`dates => 'formatted'`): \
                      12/24-hour, day and month names, quoted and escaped literals, locale prefixes, \
                      `[Red]`-style brackets (whose letters OpenSpout treats as format characters), \
                      CJK literals, every duration shape, and negative values.",
        spec: BookSpec {
            sheets: vec![SheetEntry::standard(1, "Formats")],
            has_shared_strings: false,
            num_fmts,
            cell_xfs,
            date1904: false,
        },
        sst: vec![],
        sheets: vec![xlsx::worksheet_xml(None, &rows)],
    }
}

fn all() -> Vec<Fixture> {
    vec![
        cell_types(),
        layout(),
        dates(),
        dates_1904(),
        formulas(),
        multi_sheet(),
        date_formats(),
    ]
}

/// Hand-written CSV/TSV inputs `(file, bytes, description)`, covering PHP fgetcsv quirks.
fn csv_fixtures() -> Vec<(&'static str, Vec<u8>, &'static str)> {
    let quirks = concat!(
        "\n",               // blank first line
        "id,name,note\r\n", // CRLF
        "1,plain,\"quoted, with comma\"\r\n",
        "2,\"say \"\"hi\"\"\",\"multi\nline\"\n", // doubled quotes + embedded LF
        "3,  \"spaced\",\"after\"text\n",         // space before quote, text after quote
        "4,=SUM(A1:A3),=== Totals ===\n",         // text starting with '='
        "\n",
        "\n", // blank lines mid-file
        "5,café 🚀,\"crlf\r\ninside\"\r\n",
        "6,a\rb,\" \"\n", // bare CR inside a line; quoted space
        "7,,\n",          // trailing empty fields
        "   \n",          // whitespace-only line (not blank)
        ",\n",            // two empty fields
        "\"\"\n",         // one empty quoted field
        "8,last\n",
        "\n", // blank last line
    );
    vec![
        (
            "quirks.csv",
            quirks.as_bytes().to_vec(),
            "fgetcsv edge cases: blank lines, CRLF, doubled quotes, multi-line fields, \
             whitespace before quotes, text after quotes, `=` text, a bare CR, empty fields.",
        ),
        (
            "semicolon-bom.csv",
            b"\xEF\xBB\xBFa;b;c\n1;\"x;y\";3\n".to_vec(),
            "UTF-8 BOM and a `;` delimiter (read with `csv.delimiter = ';'`).",
        ),
        (
            "unterminated.csv",
            b"a,b\n1,\"open quote\nruns to\nthe end\n".to_vec(),
            "An unterminated quote: the field runs to end of file.",
        ),
        (
            "no-final-newline.csv",
            b"a,b\n1,2".to_vec(),
            "No newline after the last record.",
        ),
        (
            "latin1-bytes.csv",
            b"name\ncaf\xE9\nna\xEFve\n".to_vec(),
            "Invalid UTF-8 (Latin-1 bytes), passed through unchanged as with OpenSpout's UTF-8 default.",
        ),
        (
            "cr-only.csv",
            b"a,b\rc,d\re,f\r".to_vec(),
            "Old Mac line endings: a bare CR is not a line break for fgetcsv, so this is one record.",
        ),
        (
            "tabs.tsv",
            b"id\tname\tnote\n1\t\"tab\there\"\tx\n2\t\tz\n".to_vec(),
            "Tab-separated, with a quoted tab (read with `csv.delimiter = \"\\t\"`).",
        ),
    ]
}

pub fn generate(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut readme = String::from(
        "# Edge-case fixtures\n\n\
         Generated by `cargo run -p gen_fixture -- edge`. Don't edit by hand; change \
         `tools/gen_fixture/src/edge.rs` and regenerate.\n\n\
         Each file targets one area of reader behaviour. The expected output is whatever \
         OpenSpout (through sheet-stream's normalisation) produces; see the parity tests.\n\n\
         | File | Covers |\n|---|---|\n",
    );
    for fx in all() {
        assert_eq!(fx.spec.sheets.len(), fx.sheets.len(), "{}", fx.file);
        let path = dir.join(fx.file);
        let mut pkg = Package::create(&path)?;
        pkg.write_book_parts(&fx.spec)?;
        if fx.spec.has_shared_strings {
            pkg.write_part("xl/sharedStrings.xml", &xlsx::shared_strings_xml(&fx.sst))?;
        }
        for (entry, xml) in fx.spec.sheets.iter().zip(&fx.sheets) {
            pkg.write_part(&entry.part, xml)?;
        }
        pkg.finish()?;
        readme.push_str(&format!("| `{}` | {} |\n", fx.file, fx.description));
        eprintln!("wrote {}", path.display());
    }
    for (file, bytes, description) in csv_fixtures() {
        let path = dir.join(file);
        std::fs::write(&path, bytes)?;
        readme.push_str(&format!("| `{file}` | {description} |\n"));
        eprintln!("wrote {}", path.display());
    }
    std::fs::write(dir.join("README.md"), readme)
}
