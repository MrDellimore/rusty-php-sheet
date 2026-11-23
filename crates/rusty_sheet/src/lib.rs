//! PHP binding layer. Keep logic in `rusty_sheet_core`; this crate only converts types.

mod convert;
mod exceptions;
mod options;
mod reader;
mod writer;

use ext_php_rs::prelude::*;

/// `rusty_sheet_version(): string`
#[php_function]
pub fn rusty_sheet_version() -> String {
    rusty_sheet_core::version().to_string()
}

#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    module
        // Parents before children.
        .class::<exceptions::SheetException>()
        .class::<exceptions::UnsupportedFormatException>()
        .class::<exceptions::ReaderException>()
        .class::<exceptions::WriterException>()
        .class::<reader::Reader>()
        .class::<writer::PhpCell>()
        .class::<writer::Writer>()
        .function(wrap_function!(rusty_sheet_version))
}
