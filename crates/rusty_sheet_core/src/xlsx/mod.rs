//! Streaming XLSX reader.
//!
//! ```no_run
//! use rusty_sheet_core::xlsx::{ReadOptions, Row, XlsxReader};
//!
//! let mut book = XlsxReader::open("big.xlsx".as_ref())?;
//! let mut rows = book.rows(0, ReadOptions::default())?;
//! let mut row = Row::default();
//! while rows.next_row(&mut row)? {
//!     println!("{}: {} cells", row.number, row.cells.len());
//! }
//! # Ok::<(), rusty_sheet_core::Error>(())
//! ```

pub mod dates;
mod formula;
pub mod numfmt;
mod package;
pub mod php_date;
#[cfg(test)]
mod read_paths;
mod shared_strings;
mod sheet;
mod styles;
mod unescape;
mod workbook;
mod xml;

use std::path::Path;
use std::sync::Arc;

pub use shared_strings::SharedStrings;
pub use sheet::{ReadOptions, Row, SheetRows};
pub use styles::{CellFormat, Styles};
pub use workbook::SheetInfo;

/// An A1-style reference for a 1-based row and 0-based column, prefixed with the
/// sheet name (quoted when Excel would quote it): `Sheet1!D8`, `'My Sheet'!A1`.
pub fn cell_ref(sheet: &str, row: u32, col: usize) -> String {
    let cell = format!("{}{row}", formula::col_letters(col as i64 + 1));
    if sheet.is_empty() {
        return cell;
    }
    if sheet
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        format!("{sheet}!{cell}")
    } else {
        format!("'{}'!{cell}", sheet.replace('\'', "''"))
    }
}

use package::Package;
use workbook::Workbook;

use crate::{Error, Result};

pub struct XlsxReader {
    pkg: Package,
    workbook: Workbook,
    /// Loaded on first use, so listing sheet names stays cheap.
    shared: Option<Arc<SharedStrings>>,
    styles: Option<Arc<Styles>>,
}

impl XlsxReader {
    /// Open an XLSX file. Reads only the zip directory and the workbook parts;
    /// shared strings and styles are loaded when the first sheet is read.
    pub fn open(path: &Path) -> Result<Self> {
        Self::from_package(Package::open(path)?)
    }

    /// Open an XLSX held in memory (mainly for tests).
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>) -> Result<Self> {
        Self::from_package(Package::from_bytes(bytes)?)
    }

    fn from_package(pkg: Package) -> Result<Self> {
        let workbook = Workbook::read(&pkg)?;
        Ok(XlsxReader {
            pkg,
            workbook,
            shared: None,
            styles: None,
        })
    }

    pub fn sheets(&self) -> &[SheetInfo] {
        &self.workbook.sheets
    }

    pub fn sheet_names(&self) -> impl Iterator<Item = &str> {
        self.workbook.sheets.iter().map(|s| s.name.as_str())
    }

    pub fn sheet_index(&self, name: &str) -> Option<usize> {
        self.workbook.sheets.iter().position(|s| s.name == name)
    }

    pub fn is_1904(&self) -> bool {
        self.workbook.date1904
    }

    /// Stream the rows of sheet `index` (0-based, workbook order). The returned
    /// stream is independent of `self`: it owns its own file handle.
    pub fn rows(&mut self, index: usize, opts: ReadOptions) -> Result<SheetRows> {
        let info = self
            .workbook
            .sheets
            .get(index)
            .ok_or_else(|| Error::SheetNotFound(format!("index {index}")))?;
        let part = info.part.clone();
        let shared = self.shared_strings()?;
        let styles = self.styles()?;
        let stream = match &part {
            Some(p) if self.pkg.contains(p) => Some(self.pkg.open_part(p)?),
            _ => None, // chartsheet or missing part: reads as empty
        };
        Ok(SheetRows::new(
            stream,
            part.unwrap_or_default(),
            shared,
            styles,
            self.workbook.date1904,
            opts,
        ))
    }

    pub fn shared_strings(&mut self) -> Result<Arc<SharedStrings>> {
        if let Some(s) = &self.shared {
            return Ok(Arc::clone(s));
        }
        let table = match &self.workbook.shared_strings_part {
            Some(part) => SharedStrings::read(&self.pkg, part)?,
            None => SharedStrings::default(),
        };
        Ok(Arc::clone(self.shared.insert(Arc::new(table))))
    }

    pub fn styles(&mut self) -> Result<Arc<Styles>> {
        if let Some(s) = &self.styles {
            return Ok(Arc::clone(s));
        }
        let styles = match &self.workbook.styles_part {
            Some(part) => Styles::read(&self.pkg, part)?,
            None => Styles::default(),
        };
        Ok(Arc::clone(self.styles.insert(Arc::new(styles))))
    }
}

#[cfg(test)]
mod tests {
    use super::cell_ref;

    #[test]
    fn cell_refs_quote_sheet_names_like_excel() {
        assert_eq!(cell_ref("Sheet1", 8, 3), "Sheet1!D8");
        assert_eq!(cell_ref("Sheet1", 1, 26), "Sheet1!AA1");
        assert_eq!(cell_ref("My Sheet", 1, 0), "'My Sheet'!A1");
        assert_eq!(cell_ref("Bob's", 2, 1), "'Bob''s'!B2");
        assert_eq!(cell_ref("", 3, 0), "A3");
    }
}
