#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| apeiron_core::fuzz::pack_entry(data));
