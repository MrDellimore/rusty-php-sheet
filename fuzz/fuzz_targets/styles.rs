#![no_main]
//! See `rusty_sheet_core::fuzzing::styles`.
libfuzzer_sys::fuzz_target!(|data: &[u8]| rusty_sheet_core::fuzzing::styles(data));
