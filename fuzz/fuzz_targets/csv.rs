#![no_main]
//! See `rusty_sheet_core::fuzzing::csv`.
libfuzzer_sys::fuzz_target!(|data: &[u8]| rusty_sheet_core::fuzzing::csv(data));
