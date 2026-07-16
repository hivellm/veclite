//! Golden-file drift test for the C ABI header (SPEC-008 FFI-006).
//!
//! Regenerates `veclite.h` from the crate's `extern "C"` surface with the exact
//! `cbindgen.toml` config the release uses, then diffs it against the committed
//! golden file. Any change to the exported surface that isn't reflected in the
//! committed header fails the build — the header cannot silently drift from the
//! code, and a reviewer sees the ABI delta in the same PR.
//!
//! To update the header after an intentional (additive) change:
//!   cbindgen --config crates/veclite-ffi/cbindgen.toml \
//!            --crate veclite-ffi --output crates/veclite-ffi/veclite.h
//! and bump `vl_abi_version()` if the change is additive to the ABI.

use std::path::PathBuf;

/// Normalize line endings so a CRLF checkout on Windows compares equal to the
/// LF the generator emits.
fn lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[test]
fn committed_header_matches_generated() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Ok(config) = cbindgen::Config::from_file(crate_dir.join("cbindgen.toml")) else {
        panic!("cbindgen.toml must parse");
    };

    let Ok(bindings) = cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    else {
        panic!("cbindgen must generate the header from the crate surface");
    };

    let mut generated = Vec::new();
    bindings.write(&mut generated);
    let generated = match String::from_utf8(generated) {
        Ok(s) => lf(&s),
        Err(e) => panic!("header is not UTF-8: {e}"),
    };

    let golden = match std::fs::read_to_string(crate_dir.join("veclite.h")) {
        Ok(s) => lf(&s),
        Err(e) => panic!("committed veclite.h must exist: {e}"),
    };

    if generated != golden {
        // Surface the first differing line to make the drift obvious.
        let first_diff = generated
            .lines()
            .zip(golden.lines())
            .enumerate()
            .find(|(_, (g, c))| g != c)
            .map_or_else(
                || {
                    format!(
                        "length differs: generated {} lines, committed {} lines",
                        generated.lines().count(),
                        golden.lines().count()
                    )
                },
                |(i, (g, c))| format!("line {}:\n  generated: {g}\n  committed: {c}", i + 1),
            );
        panic!(
            "veclite.h is out of sync with the extern \"C\" surface.\n{first_diff}\n\n\
             Regenerate:\n  cbindgen --config crates/veclite-ffi/cbindgen.toml \
             --crate veclite-ffi --output crates/veclite-ffi/veclite.h\n\
             and bump vl_abi_version() if the ABI changed."
        );
    }
}
