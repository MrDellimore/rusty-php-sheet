//! PHP exception hierarchy:
//!
//! ```text
//! \Exception
//! └── RustySheet\Exception
//!     ├── RustySheet\UnsupportedFormatException   (callers fall back to OpenSpout)
//!     ├── RustySheet\ReaderException
//!     └── RustySheet\WriterException              (Phase 5)
//! ```
//! Invalid options throw PHP's built-in `\ValueError`.

use ext_php_rs::exception::PhpException;
use ext_php_rs::flags::ClassFlags;
use ext_php_rs::prelude::*;
use ext_php_rs::zend::ce;

use rusty_sheet_core::Error;

#[php_class]
#[php(name = "RustySheet\\Exception")]
// Extends \Exception rather than SPL's \RuntimeException: SPL classes can't be
// looked up during module startup, when these classes are registered.
#[php(extends(ce = ce::exception, stub = "\\Exception"))]
pub struct SheetException;

#[php_class]
#[php(name = "RustySheet\\UnsupportedFormatException")]
#[php(extends(SheetException))]
#[php(flags = ClassFlags::Final)]
pub struct UnsupportedFormatException;

#[php_class]
#[php(name = "RustySheet\\ReaderException")]
#[php(extends(SheetException))]
#[php(flags = ClassFlags::Final)]
pub struct ReaderException;

#[php_class]
#[php(name = "RustySheet\\WriterException")]
#[php(extends(SheetException))]
#[php(flags = ClassFlags::Final)]
pub struct WriterException;

pub fn unsupported(message: impl Into<String>) -> PhpException {
    PhpException::from_class::<UnsupportedFormatException>(message.into())
}

pub fn reader_error(message: impl Into<String>) -> PhpException {
    PhpException::from_class::<ReaderException>(message.into())
}

pub fn invalid_option(name: &str, message: &str) -> PhpException {
    PhpException::new(
        format!("Invalid RustySheet option `{name}`: {message}"),
        0,
        ce::value_error(),
    )
}

/// Map a core error to the right PHP exception, prefixed with the file path.
pub fn from_core(err: Error, path: &str) -> PhpException {
    match err {
        Error::UnsupportedFormat(msg) => unsupported(format!("{path}: unsupported format ({msg})")),
        other => reader_error(format!("{path}: {other}")),
    }
}
