//! Pure-Rust core: streaming XLSX/CSV reading and writing.
//!
//! Kept free of PHP types so it can be unit-tested and benchmarked with plain
//! `cargo test` / `cargo bench`. The `rusty_sheet` crate converts these types to PHP values.

pub mod csv;
pub mod error;
pub mod value;
pub mod writer;
pub mod xlsx;

pub use error::{Error, Result};
pub use value::{CellValue, DateKind, NumFmtId};

/// Library version, exposed to PHP for diagnostics.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }
}
