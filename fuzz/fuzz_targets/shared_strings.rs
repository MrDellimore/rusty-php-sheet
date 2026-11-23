#![no_main]
//! See `rusty_sheet_core::fuzzing::shared_strings`.
libfuzzer_sys::fuzz_target!(|data: &[u8]| rusty_sheet_core::fuzzing::shared_strings(data));
