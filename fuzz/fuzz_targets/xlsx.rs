#![no_main]
//! A whole XLSX package: the zip container, workbook discovery and every part.
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = rusty_sheet_core::fuzzing::read_xlsx(data);
});
