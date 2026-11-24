//! Data validation, cell comments (notes) and tables: what they are and the XML
//! they produce. Validation and comments are written as OpenSpout 5.15 writes them
//! (`FileSystemHelper::createContentFiles`, `CommentsManager`); tables, which
//! OpenSpout doesn't have, as PhpSpreadsheet 5.11 writes them (`Writer\Xlsx\Table`).

use std::fmt::Write as _;

use super::escape::escape;
use super::xlsx::CellRange;
use crate::{Error, Result};

// ---- data validation --------------------------------------------------------------

/// One `<dataValidation>` rule (OpenSpout's `ValidationRule` + `ValidationDisplay`).
#[derive(Debug, Clone, PartialEq)]
pub struct Validation {
    /// whole, decimal, date, time, textLength, list or custom.
    pub kind: String,
    pub operator: Option<String>,
    pub formula1: Formula,
    pub formula2: Option<Formula>,
    pub allow_blank: bool,
    pub show_input_message: bool,
    pub show_error_message: bool,
    /// stop, warning or information.
    pub error_style: String,
    pub prompt_title: Option<String>,
    pub prompt: Option<String>,
    pub error_title: Option<String>,
    pub error: Option<String>,
}

/// A formula as it goes into `<formula1>`/`<formula2>`.
#[derive(Debug, Clone, PartialEq)]
pub enum Formula {
    /// An inline list: `"a,b,c"`.
    List(Vec<String>),
    /// Anything else: a number, a reference, a formula. Written with `&`, `<` and
    /// `>` escaped (PHP's `htmlspecialchars(…, ENT_XML1)`).
    Text(String),
}

/// Excel refuses (repairs) an inline list longer than this.
pub const MAX_LIST_CHARS: usize = 255;

impl Validation {
    /// The kinds, operators and styles Excel knows; checked so a bad value can't
    /// produce a file Excel has to repair.
    pub fn check(&self) -> Result<()> {
        let bad = |what: &str| Err(Error::InvalidArgument(what.to_string()));
        if !matches!(
            self.kind.as_str(),
            "whole" | "decimal" | "date" | "time" | "textLength" | "list" | "custom"
        ) {
            return bad(&format!("unknown validation type '{}'", self.kind));
        }
        if let Some(op) = &self.operator
            && !matches!(
                op.as_str(),
                "between"
                    | "notBetween"
                    | "equal"
                    | "notEqual"
                    | "greaterThan"
                    | "lessThan"
                    | "greaterThanOrEqual"
                    | "lessThanOrEqual"
            )
        {
            return bad(&format!("unknown validation operator '{op}'"));
        }
        if matches!(self.operator.as_deref(), Some("between" | "notBetween"))
            && self.formula2.is_none()
        {
            return bad("between and notBetween need a second value");
        }
        if !matches!(
            self.error_style.as_str(),
            "stop" | "warning" | "information"
        ) {
            return bad(&format!("unknown error style '{}'", self.error_style));
        }
        if let Formula::List(values) = &self.formula1 {
            if let Some(v) = values.iter().find(|v| v.contains(',')) {
                return bad(&format!(
                    "list value '{v}' contains a comma, which separates the values"
                ));
            }
            let chars: usize = values.iter().map(|v| v.chars().count()).sum::<usize>()
                + values.len().saturating_sub(1);
            if chars > MAX_LIST_CHARS {
                return bad(&format!(
                    "an inline list can be at most {MAX_LIST_CHARS} characters (Excel's limit); put the values in cells and use a source range"
                ));
            }
        }
        Ok(())
    }

    /// OpenSpout's `<dataValidation …>` element for `range`.
    pub fn xml(&self, range: &CellRange, out: &mut String) {
        let flag = |b: bool| u8::from(b);
        let attr = |name: &str, v: &Option<String>| {
            v.as_ref()
                .map(|v| format!(" {name}=\"{}\"", escape(v)))
                .unwrap_or_default()
        };
        let _ = write!(
            out,
            "<dataValidation type=\"{}\"{} allowBlank=\"{}\" showInputMessage=\"{}\" showErrorMessage=\"{}\" errorStyle=\"{}\"{}{}{}{} sqref=\"{range}\"><formula1>",
            self.kind,
            self.operator
                .as_ref()
                .map(|o| format!(" operator=\"{o}\""))
                .unwrap_or_default(),
            flag(self.allow_blank),
            flag(self.show_input_message),
            flag(self.show_error_message),
            self.error_style,
            attr("promptTitle", &self.prompt_title),
            attr("prompt", &self.prompt),
            attr("errorTitle", &self.error_title),
            attr("error", &self.error),
        );
        self.formula1.xml(out);
        out.push_str("</formula1>");
        if let Some(f2) = &self.formula2 {
            out.push_str("<formula2>");
            f2.xml(out);
            out.push_str("</formula2>");
        }
        out.push_str("</dataValidation>");
    }
}

impl Formula {
    fn xml(&self, out: &mut String) {
        match self {
            Formula::List(values) => {
                out.push('"');
                for (i, v) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&escape_xml1(v));
                }
                out.push('"');
            }
            Formula::Text(t) => out.push_str(&escape_xml1(t)),
        }
    }
}

/// `htmlspecialchars($s, ENT_XML1)`: only `&`, `<` and `>` (quotes stay as they are).
fn escape_xml1(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

// ---- comments ----------------------------------------------------------------------

/// A cell comment (a note): OpenSpout's `Comment` with its `TextRun`s.
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub runs: Vec<TextRun>,
    pub visible: bool,
    pub width: String,
    pub height: String,
    pub margin_left: String,
    pub margin_top: String,
    /// `#RRGGBB`.
    pub fill: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub size: i64,
    pub color: String,
    pub font: String,
}

impl Default for Comment {
    /// OpenSpout's defaults.
    fn default() -> Self {
        Comment {
            runs: Vec::new(),
            visible: false,
            width: "96pt".into(),
            height: "55.5pt".into(),
            margin_left: "59.25pt".into(),
            margin_top: "1.5pt".into(),
            fill: "#FFFFE1".into(),
        }
    }
}

impl TextRun {
    /// A run in OpenSpout's defaults: Tahoma 10, black.
    pub fn plain(text: &str) -> Self {
        TextRun {
            text: text.to_string(),
            bold: false,
            italic: false,
            size: 10,
            color: "000000".into(),
            font: "Tahoma".into(),
        }
    }
}

/// `CommentsManager::COMMENTS_XML_FILE_HEADER` / `FOOTER`.
pub const COMMENTS_HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<comments xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\n    \
<authors><author>Unknown</author></authors>\n    <commentList>";
pub const COMMENTS_FOOTER: &str = "    </commentList>\n</comments>";

/// `CommentsManager::DRAWINGS_VML_FILE_HEADER` / `FOOTER`.
pub const VML_HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\">\n  \
<o:shapelayout v:ext=\"edit\">\n    <o:idmap v:ext=\"edit\" data=\"1\"/>\n  </o:shapelayout>\n  \
<v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\">\n    \
<v:stroke joinstyle=\"miter\"/>\n    <v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/>\n  </v:shapetype>";
pub const VML_FOOTER: &str = "</xml>";

impl Comment {
    /// `CommentsManager::addXmlComment`.
    pub fn comment_xml(&self, cell_ref: &str, out: &mut String) {
        let _ = write!(out, "<comment ref=\"{cell_ref}\" authorId=\"0\"><text>");
        for r in &self.runs {
            out.push_str("<r>  <rPr>");
            if r.bold {
                out.push_str("    <b/>");
            }
            if r.italic {
                out.push_str("    <i/>");
            }
            let _ = write!(
                out,
                "    <sz val=\"{}\"/>    <color rgb=\"{}\"/>    <rFont val=\"{}\"/>    <family val=\"2\"/>  </rPr>  <t xml:space=\"preserve\">{}</t></r>",
                r.size,
                escape(&r.color),
                escape(&r.font),
                escape(&r.text)
            );
        }
        out.push_str("</text></comment>");
    }

    /// `CommentsManager::addVmlComment`; `shape_id` counts across the workbook.
    pub fn vml(&self, shape_id: u32, row0: u32, col0: u32, out: &mut String) {
        let hidden = if self.visible {
            ""
        } else {
            ";visibility:hidden"
        };
        let _ = write!(
            out,
            "<v:shape id=\"_x0000_s{shape_id}\" type=\"#_x0000_t202\" style=\"position:absolute;z-index:1;margin-left:{};margin-top:{};width:{};height:{}{hidden}\" fillcolor=\"{}\" o:insetmode=\"auto\">\
             <v:fill color2=\"{}\"/><v:shadow on=\"t\" color=\"black\" obscured=\"t\"/><v:path o:connecttype=\"none\"/>\
             <v:textbox style=\"mso-direction-alt:auto\">  <div style=\"text-align:left\"/></v:textbox>\
             <x:ClientData ObjectType=\"Note\">  <x:MoveWithCells/>  <x:SizeWithCells/>  <x:AutoFill>False</x:AutoFill>  \
             <x:Row>{row0}</x:Row>  <x:Column>{col0}</x:Column></x:ClientData></v:shape>",
            escape(&self.margin_left),
            escape(&self.margin_top),
            escape(&self.width),
            escape(&self.height),
            escape(&self.fill),
            escape(&self.fill),
        );
    }
}

// ---- tables ------------------------------------------------------------------------

/// PhpSpreadsheet's `Table` and `TableStyle` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct TableOptions {
    pub style: String,
    pub header_row: bool,
    pub filter: bool,
    pub row_stripes: bool,
    pub column_stripes: bool,
    pub first_column: bool,
    pub last_column: bool,
}

impl Default for TableOptions {
    fn default() -> Self {
        TableOptions {
            style: "TableStyleMedium2".into(),
            header_row: true,
            filter: true,
            row_stripes: false,
            column_stripes: false,
            first_column: false,
            last_column: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Table {
    pub range: CellRange,
    pub name: String,
    pub options: TableOptions,
    /// Header texts captured from the header row, by column offset.
    pub headers: Vec<Option<String>>,
}

impl Table {
    /// Its header row shows filter dropdowns.
    pub fn filters_header(&self) -> bool {
        self.options.header_row && self.options.filter
    }

    /// Column names: the header texts, "ColumnN" where there is none (or no header
    /// row), and a number appended to a repeat (Excel needs them unique).
    pub fn column_names(&self) -> Vec<String> {
        let count = (self.range.last_col - self.range.first_col + 1) as usize;
        let mut names: Vec<String> = Vec::with_capacity(count);
        for i in 0..count {
            let base = self
                .headers
                .get(i)
                .cloned()
                .flatten()
                .filter(|h| self.options.header_row && !h.trim().is_empty())
                .unwrap_or_else(|| format!("Column{}", i + 1));
            let mut name = base.clone();
            let mut n = 2;
            while names
                .iter()
                .any(|x| x.to_lowercase() == name.to_lowercase())
            {
                name = format!("{base}{n}");
                n += 1;
            }
            names.push(name);
        }
        names
    }

    /// `Writer\Xlsx\Table::writeTable` for table number `id`.
    pub fn xml(&self, id: u32) -> String {
        let range = self.range;
        let mut s = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
             <table xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" id=\"{id}\" name=\"Table{id}\" displayName=\"{}\" ref=\"{range}\" headerRowCount=\"{}\" totalsRowCount=\"0\">",
            escape(&self.name),
            u8::from(self.options.header_row),
        );
        if self.filters_header() {
            let _ = write!(s, "<autoFilter ref=\"{range}\"/>");
        }
        let names = self.column_names();
        let _ = write!(s, "<tableColumns count=\"{}\">", names.len());
        for (i, n) in names.iter().enumerate() {
            let _ = write!(s, "<tableColumn id=\"{}\" name=\"{}\"/>", i + 1, escape(n));
        }
        let o = &self.options;
        let _ = write!(
            s,
            "</tableColumns><tableStyleInfo name=\"{}\" showFirstColumn=\"{}\" showLastColumn=\"{}\" showRowStripes=\"{}\" showColumnStripes=\"{}\"/></table>",
            escape(&o.style),
            u8::from(o.first_column),
            u8::from(o.last_column),
            u8::from(o.row_stripes),
            u8::from(o.column_stripes),
        );
        s
    }
}

/// PhpSpreadsheet's `Table::setName` rules: a letter, `_` or `\` first, then letters,
/// marks, digits, `_` and `.`; at most 255 characters; not a cell reference.
pub fn check_table_name(name: &str) -> Result<()> {
    let bad = |why: &str| {
        Err(Error::InvalidArgument(format!(
            "invalid table name '{name}': {why}"
        )))
    };
    if name.is_empty() {
        return bad("it is empty");
    }
    if name.chars().count() > 255 {
        return bad("it is longer than 255 characters");
    }
    if matches!(name, "C" | "c" | "R" | "r") || is_cell_reference(name) {
        return bad("it looks like a cell reference");
    }
    let mut chars = name.chars();
    let first = chars.next().expect("not empty");
    if !(first.is_alphabetic() || first == '_' || first == '\\') {
        return bad("it must start with a letter, an underscore or a backslash");
    }
    if !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
        return bad("only letters, digits, underscores and periods are allowed");
    }
    Ok(())
}

/// `A1`, `$AB$12` or R1C1 notation (`R1C1`, `R[2]C`, `RC[-1]`).
fn is_cell_reference(name: &str) -> bool {
    if super::xlsx::CellRange::parse(name).is_some() && !name.contains(':') {
        return true;
    }
    let upper = name.to_ascii_uppercase();
    let Some(rest) = upper.strip_prefix('R') else {
        return false;
    };
    let skip_part = |s: &str| -> Option<usize> {
        // digits, or [ -digits ], possibly empty
        let b = s.as_bytes();
        if b.first() == Some(&b'[') {
            let close = s.find(']')?;
            let inner = &s[1..close];
            let inner = inner.strip_prefix('-').unwrap_or(inner);
            (!inner.is_empty() && inner.bytes().all(|c| c.is_ascii_digit())).then_some(close + 1)
        } else {
            Some(s.bytes().take_while(u8::is_ascii_digit).count())
        }
    };
    let Some(n) = skip_part(rest) else {
        return false;
    };
    let Some(rest) = rest[n..].strip_prefix('C') else {
        return false;
    };
    skip_part(rest).is_some_and(|m| m == rest.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(kind: &str) -> Validation {
        Validation {
            kind: kind.into(),
            operator: None,
            formula1: Formula::Text("1".into()),
            formula2: None,
            allow_blank: true,
            show_input_message: true,
            show_error_message: true,
            error_style: "stop".into(),
            prompt_title: None,
            prompt: None,
            error_title: None,
            error: None,
        }
    }

    #[test]
    fn validation_xml_like_openspout() {
        let range = CellRange::parse("B2:B10").unwrap();
        let mut list = rule("list");
        list.formula1 = Formula::List(vec!["Yes".into(), "No & maybe".into()]);
        list.error_title = Some("Oops \"x\"".into());
        let mut s = String::new();
        list.xml(&range, &mut s);
        assert_eq!(
            s,
            "<dataValidation type=\"list\" allowBlank=\"1\" showInputMessage=\"1\" showErrorMessage=\"1\" errorStyle=\"stop\" errorTitle=\"Oops &quot;x&quot;\" sqref=\"B2:B10\"><formula1>\"Yes,No &amp; maybe\"</formula1></dataValidation>"
        );
        let mut between = rule("whole");
        between.operator = Some("between".into());
        between.formula2 = Some(Formula::Text("10".into()));
        between.error_style = "warning".into();
        s.clear();
        between.xml(&range, &mut s);
        assert!(s.contains(" operator=\"between\" allowBlank"), "{s}");
        assert!(s.ends_with("<formula1>1</formula1><formula2>10</formula2></dataValidation>"));
    }

    #[test]
    fn validation_checks() {
        let err = |v: Validation| v.check().unwrap_err().to_string();
        assert!(err(rule("weird")).contains("unknown validation type"));
        let mut v = rule("whole");
        v.operator = Some("about".into());
        assert!(err(v.clone()).contains("unknown validation operator"));
        v.operator = Some("notBetween".into());
        assert!(err(v.clone()).contains("second value"));
        let mut v = rule("list");
        v.error_style = "panic".into();
        assert!(err(v).contains("unknown error style"));
        let mut v = rule("list");
        v.formula1 = Formula::List(vec!["a,b".into()]);
        assert!(err(v.clone()).contains("comma"));
        v.formula1 = Formula::List(vec!["x".repeat(200), "y".repeat(55)]);
        assert!(err(v.clone()).contains("at most 255"));
        v.formula1 = Formula::List(vec!["x".repeat(200), "y".repeat(54)]);
        assert!(v.check().is_ok());
    }

    #[test]
    fn table_names() {
        for ok in [
            "Claims",
            "_x",
            "\\x",
            "Tbl.2025_q1",
            "Lïst",
            "RC1x",
            "ABC1D",
            "Rabc",
        ] {
            assert!(check_table_name(ok).is_ok(), "{ok}");
        }
        // Not R1C1 references: an unclosed bracket, and no C after the row part.
        assert!(!is_cell_reference("R[5"));
        assert!(!is_cell_reference("Rabc"));
        assert!(is_cell_reference("Rx1")); // column RX, row 1
        for bad in [
            "",
            "1abc",
            "a b",
            "a-b",
            "A1",
            "xfd1048576",
            "R1C1",
            "R[2]C",
            "RC[-1]",
            "c",
            "R",
            &"x".repeat(256),
        ] {
            assert!(check_table_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn column_names_are_unique_and_never_empty() {
        let t = Table {
            range: CellRange::parse("A1:E3").unwrap(),
            name: "T".into(),
            options: TableOptions::default(),
            headers: vec![
                Some("Name".into()),
                None,
                Some("name".into()),
                Some(" ".into()),
            ],
        };
        assert_eq!(
            t.column_names(),
            ["Name", "Column2", "name2", "Column4", "Column5"]
        );
        let no_header = Table {
            options: TableOptions {
                header_row: false,
                ..TableOptions::default()
            },
            ..t
        };
        assert_eq!(no_header.column_names()[0], "Column1");
        assert!(!no_header.filters_header());
        assert!(!no_header.xml(1).contains("<autoFilter"));
    }
}
