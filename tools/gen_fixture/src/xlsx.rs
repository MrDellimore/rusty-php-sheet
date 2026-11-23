//! Minimal hand-rolled XLSX package writer.
//!
//! We write the XML ourselves (rather than using an XLSX library) so fixtures can
//! contain exactly the constructs a reader must cope with: shared strings, rich
//! text, missing `r` attributes, odd relationship targets, 1904 dates, etc.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use zip::ZipWriter;
use zip::write::SimpleFileOptions;

pub const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
pub const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_WORKSHEET: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const REL_STYLES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const REL_SST: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";
const REL_OFFICE_DOC: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

/// One worksheet entry in the workbook.
pub struct SheetEntry {
    pub name: String,
    /// Zip part name, e.g. `xl/worksheets/sheet1.xml`.
    pub part: String,
    /// Target as written in `workbook.xml.rels` (relative to `xl/`, or absolute with `/`).
    pub rel_target: String,
    pub rel_id: String,
}

impl SheetEntry {
    /// Conventional entry: `xl/worksheets/sheet{n}.xml`, rel id `rId{n}`.
    pub fn standard(n: usize, name: &str) -> Self {
        SheetEntry {
            name: name.to_string(),
            part: format!("xl/worksheets/sheet{n}.xml"),
            rel_target: format!("worksheets/sheet{n}.xml"),
            rel_id: format!("rId{n}"),
        }
    }
}

/// Workbook-level description: everything except the sheet bodies.
pub struct BookSpec {
    pub sheets: Vec<SheetEntry>,
    pub has_shared_strings: bool,
    /// Custom number formats `(numFmtId, formatCode)`; ids should be >= 164.
    pub num_fmts: Vec<(u32, String)>,
    /// `numFmtId` for each `cellXfs` entry; the cell attribute `s="i"` indexes this.
    pub cell_xfs: Vec<u32>,
    pub date1904: bool,
}

pub struct Package {
    zip: ZipWriter<BufWriter<File>>,
    opts: SimpleFileOptions,
}

impl Package {
    pub fn create(path: &Path) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = BufWriter::with_capacity(1 << 20, File::create(path)?);
        // Fixed timestamp so committed fixtures are byte-for-byte reproducible.
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        Ok(Package {
            zip: ZipWriter::new(file),
            opts,
        })
    }

    /// Start a part; write its content through the returned writer.
    pub fn start_part(&mut self, name: &str) -> io::Result<&mut impl Write> {
        self.zip
            .start_file(name, self.opts)
            .map_err(io::Error::other)?;
        Ok(&mut self.zip)
    }

    pub fn write_part(&mut self, name: &str, content: &str) -> io::Result<()> {
        self.start_part(name)?.write_all(content.as_bytes())
    }

    /// Write every workbook-level part (content types, rels, workbook, styles).
    pub fn write_book_parts(&mut self, spec: &BookSpec) -> io::Result<()> {
        self.write_part("[Content_Types].xml", &content_types(spec))?;
        self.write_part("_rels/.rels", &root_rels())?;
        self.write_part("xl/workbook.xml", &workbook_xml(spec))?;
        self.write_part("xl/_rels/workbook.xml.rels", &workbook_rels(spec))?;
        self.write_part("xl/styles.xml", &styles_xml(spec))
    }

    pub fn finish(self) -> io::Result<()> {
        let mut inner = self.zip.finish().map_err(io::Error::other)?;
        inner.flush()
    }
}

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Column index (0-based) to letters: 0 → A, 25 → Z, 26 → AA.
pub fn col_letters(mut idx: usize) -> String {
    let mut buf = Vec::new();
    loop {
        buf.push(b'A' + (idx % 26) as u8);
        if idx < 26 {
            break;
        }
        idx = idx / 26 - 1;
    }
    buf.reverse();
    String::from_utf8(buf).expect("ASCII")
}

/// Wrap `<sheetData>` content in a complete worksheet document.
pub fn worksheet_xml(dimension: Option<&str>, sheet_data_inner: &str) -> String {
    let mut s = worksheet_open(dimension);
    s.push_str(sheet_data_inner);
    s.push_str(WORKSHEET_CLOSE);
    s
}

/// Everything up to and including `<sheetData>`; pair with [`WORKSHEET_CLOSE`].
pub fn worksheet_open(dimension: Option<&str>) -> String {
    let dim = dimension
        .map(|d| format!("<dimension ref=\"{d}\"/>"))
        .unwrap_or_default();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <worksheet xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\">{dim}\
         <sheetViews><sheetView workbookViewId=\"0\"/></sheetViews>\
         <sheetFormatPr defaultRowHeight=\"15\"/><sheetData>"
    )
}

pub const WORKSHEET_CLOSE: &str = "</sheetData></worksheet>";

/// `items` are raw `<si>` inner XML fragments (already escaped).
pub fn shared_strings_xml(items: &[String]) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <sst xmlns=\"{NS_MAIN}\" count=\"{n}\" uniqueCount=\"{n}\">",
        n = items.len()
    );
    for item in items {
        s.push_str("<si>");
        s.push_str(item);
        s.push_str("</si>");
    }
    s.push_str("</sst>");
    s
}

fn content_types(spec: &BookSpec) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
         <Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>",
    );
    if spec.has_shared_strings {
        s.push_str("<Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/>");
    }
    for sheet in &spec.sheets {
        s.push_str(&format!(
            "<Override PartName=\"/{}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>",
            sheet.part
        ));
    }
    s.push_str("</Types>");
    s
}

fn root_rels() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"{REL_OFFICE_DOC}\" Target=\"xl/workbook.xml\"/>\
         </Relationships>"
    )
}

fn workbook_xml(spec: &BookSpec) -> String {
    let pr = if spec.date1904 {
        "<workbookPr date1904=\"1\"/>"
    } else {
        "<workbookPr/>"
    };
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <workbook xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\">{pr}<sheets>"
    );
    for (i, sheet) in spec.sheets.iter().enumerate() {
        s.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"{}\"/>",
            xml_escape(&sheet.name),
            i + 1,
            sheet.rel_id
        ));
    }
    s.push_str("</sheets></workbook>");
    s
}

fn workbook_rels(spec: &BookSpec) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for sheet in &spec.sheets {
        s.push_str(&format!(
            "<Relationship Id=\"{}\" Type=\"{REL_WORKSHEET}\" Target=\"{}\"/>",
            sheet.rel_id, sheet.rel_target
        ));
    }
    s.push_str(&format!(
        "<Relationship Id=\"rIdStyles\" Type=\"{REL_STYLES}\" Target=\"styles.xml\"/>"
    ));
    if spec.has_shared_strings {
        s.push_str(&format!(
            "<Relationship Id=\"rIdSst\" Type=\"{REL_SST}\" Target=\"sharedStrings.xml\"/>"
        ));
    }
    s.push_str("</Relationships>");
    s
}

fn styles_xml(spec: &BookSpec) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <styleSheet xmlns=\"{NS_MAIN}\">"
    );
    if !spec.num_fmts.is_empty() {
        s.push_str(&format!("<numFmts count=\"{}\">", spec.num_fmts.len()));
        for (id, code) in &spec.num_fmts {
            s.push_str(&format!(
                "<numFmt numFmtId=\"{id}\" formatCode=\"{}\"/>",
                xml_escape(code)
            ));
        }
        s.push_str("</numFmts>");
    }
    s.push_str(
        "<fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
         <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill>\
         <fill><patternFill patternType=\"gray125\"/></fill></fills>\
         <borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
         <cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>",
    );
    let xfs: Vec<u32> = if spec.cell_xfs.is_empty() {
        vec![0]
    } else {
        spec.cell_xfs.clone()
    };
    s.push_str(&format!("<cellXfs count=\"{}\">", xfs.len()));
    for id in xfs {
        let apply = if id == 0 {
            ""
        } else {
            " applyNumberFormat=\"1\""
        };
        s.push_str(&format!(
            "<xf numFmtId=\"{id}\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"{apply}/>"
        ));
    }
    s.push_str(
        "</cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
         </styleSheet>",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_letters() {
        assert_eq!(col_letters(0), "A");
        assert_eq!(col_letters(25), "Z");
        assert_eq!(col_letters(26), "AA");
        assert_eq!(col_letters(49), "AX");
        assert_eq!(col_letters(701), "ZZ");
        assert_eq!(col_letters(702), "AAA");
    }

    #[test]
    fn escaping() {
        assert_eq!(xml_escape("a&b<c>\"d\""), "a&amp;b&lt;c&gt;&quot;d&quot;");
    }
}
