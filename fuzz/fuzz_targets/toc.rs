//! TST-050 fuzz target: see `veclite::fuzz_api::run_toc`.
#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    veclite::fuzz_api::run_toc(data);
});
