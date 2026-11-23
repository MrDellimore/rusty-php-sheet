//! Error type shared by all readers and writers.
//!
//! Policy (implementation plan, decision D8): a bad *cell value* never produces an
//! error; it becomes `CellValue::Null`. Errors are for problems with the file or
//! its structure, which the PHP layer surfaces as `RustySheet\ReaderException`
//! (or skips the row when `on_error = 'skip_row'`).

use std::io;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("invalid zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("malformed XML in {part}: {source}")]
    Xml {
        part: String,
        #[source]
        source: quick_xml::Error,
    },

    /// A part the format requires (e.g. `xl/workbook.xml`) is absent.
    #[error("missing required part: {0}")]
    MissingPart(String),

    /// The file extension or content is not a format we read/write.
    /// Maps to `RustySheet\UnsupportedFormatException` so callers can fall back to OpenSpout.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    /// A sheet name or index that does not exist in the workbook.
    #[error("sheet not found: {0}")]
    SheetNotFound(String),

    /// Structurally invalid content that is well-formed XML (e.g. a bad cell reference).
    #[error("invalid data in {part}: {message}")]
    InvalidData { part: String, message: String },
}

impl Error {
    /// True when the caller should fall back to another engine rather than fail.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Error::UnsupportedFormat(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_is_detectable() {
        assert!(Error::UnsupportedFormat("xls".into()).is_unsupported());
        assert!(!Error::MissingPart("xl/workbook.xml".into()).is_unsupported());
    }

    #[test]
    fn messages_are_readable() {
        let e = Error::InvalidData {
            part: "xl/worksheets/sheet1.xml".into(),
            message: "bad cell reference `1A`".into(),
        };
        assert_eq!(
            e.to_string(),
            "invalid data in xl/worksheets/sheet1.xml: bad cell reference `1A`"
        );
    }
}
