//! TST-050 fuzz target: see `veclite::fuzz_api::run_image`.
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    veclite::fuzz_api::run_image(data);
});
