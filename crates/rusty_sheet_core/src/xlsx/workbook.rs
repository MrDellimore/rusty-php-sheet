//! `_rels/.rels` → workbook part → `workbook.xml` + its rels:
//! sheet names and part paths in workbook (tab) order, plus the 1904 date flag.

use std::collections::HashMap;
use std::io::BufReader;

use quick_xml::events::Event;

use super::package::{Package, resolve_target};
use super::xml;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetInfo {
    pub name: String,
    /// Part path, e.g. `xl/worksheets/sheet1.xml`. `None` if the relationship is
    /// missing or isn't a worksheet (e.g. a chartsheet); such sheets read as empty.
    pub part: Option<String>,
    /// `state="hidden"` / `"veryHidden"`.
    pub hidden: bool,
}

#[derive(Debug, Clone)]
pub struct Workbook {
    pub sheets: Vec<SheetInfo>,
    pub date1904: bool,
    pub shared_strings_part: Option<String>,
    pub styles_part: Option<String>,
}

#[derive(Debug)]
struct Relationship {
    kind: String,
    target: String,
}

impl Workbook {
    pub fn read(pkg: &Package) -> Result<Self> {
        let root_rels = read_rels(pkg, "_rels/.rels")?;
        let workbook_part = root_rels
            .values()
            .find(|r| r.kind.ends_with("/officeDocument"))
            .map(|r| resolve_target("", &r.target))
            .unwrap_or_else(|| "xl/workbook.xml".to_string());
        if !pkg.contains(&workbook_part) {
            return Err(Error::MissingPart(workbook_part));
        }

        let rels_part = rels_part_for(&workbook_part);
        let rels = if pkg.contains(&rels_part) {
            read_rels(pkg, &rels_part)?
        } else {
            HashMap::new()
        };
        let find_kind = |suffix: &str| {
            rels.values()
                .find(|r| r.kind.ends_with(suffix))
                .map(|r| resolve_target(&workbook_part, &r.target))
                .filter(|p| pkg.contains(p))
        };
        // Some writers omit the relationship but still ship the conventional part.
        let conventional = |name: &str| {
            let dir = workbook_part.rsplit_once('/').map_or("", |(d, _)| d);
            let p = if dir.is_empty() {
                name.to_string()
            } else {
                format!("{dir}/{name}")
            };
            pkg.contains(&p).then_some(p)
        };
        let shared_strings_part =
            find_kind("/sharedStrings").or_else(|| conventional("sharedStrings.xml"));
        let styles_part = find_kind("/styles").or_else(|| conventional("styles.xml"));

        let (raw_sheets, date1904) = parse_workbook(pkg, &workbook_part)?;
        let sheets = raw_sheets
            .into_iter()
            .map(|(name, rel_id, hidden)| {
                let part = rel_id
                    .and_then(|id| rels.get(&id))
                    .filter(|r| r.kind.ends_with("/worksheet"))
                    .map(|r| resolve_target(&workbook_part, &r.target));
                SheetInfo { name, part, hidden }
            })
            .collect();

        Ok(Workbook {
            sheets,
            date1904,
            shared_strings_part,
            styles_part,
        })
    }
}

/// `xl/workbook.xml` → `xl/_rels/workbook.xml.rels`
fn rels_part_for(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

fn read_rels(pkg: &Package, part: &str) -> Result<HashMap<String, Relationship>> {
    let bytes = pkg.read_part(part)?;
    let mut reader = xml::reader(BufReader::new(bytes.as_slice()));
    let mut buf = Vec::new();
    let mut out = HashMap::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(xml::xml_error(part))?
        {
            Event::Start(e) | Event::Empty(e) if xml::local_name(&e) == "Relationship" => {
                // External targets (hyperlinks etc.) aren't parts.
                if xml::attr(&e, "TargetMode").as_deref() == Some("External") {
                    continue;
                }
                if let (Some(id), Some(kind), Some(target)) = (
                    xml::attr_unescaped(&e, "Id"),
                    xml::attr_unescaped(&e, "Type"),
                    xml::attr_unescaped(&e, "Target"),
                ) {
                    out.insert(id, Relationship { kind, target });
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

type RawSheet = (String, Option<String>, bool);

fn parse_workbook(pkg: &Package, part: &str) -> Result<(Vec<RawSheet>, bool)> {
    let bytes = pkg.read_part(part)?;
    let mut reader = xml::reader(BufReader::new(bytes.as_slice()));
    let mut buf = Vec::new();
    let mut sheets = Vec::new();
    let mut date1904 = false;
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(xml::xml_error(part))?
        {
            Event::Start(e) | Event::Empty(e) => match xml::local_name(&e) {
                "workbookPr" => {
                    date1904 = matches!(
                        xml::attr(&e, "date1904").as_deref(),
                        Some("1") | Some("true")
                    );
                }
                "sheet" => {
                    let name = xml::attr_unescaped(&e, "name").unwrap_or_default();
                    let rel_id = xml::prefixed_attr(&e, "id");
                    let hidden = matches!(
                        xml::attr(&e, "state").as_deref(),
                        Some("hidden") | Some("veryHidden")
                    );
                    sheets.push((name, rel_id, hidden));
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok((sheets, date1904))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rels_part_path() {
        assert_eq!(
            rels_part_for("xl/workbook.xml"),
            "xl/_rels/workbook.xml.rels"
        );
        assert_eq!(rels_part_for("workbook.xml"), "_rels/workbook.xml.rels");
    }
}
