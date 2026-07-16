## 1. Implementation
- [x] 1.1 Context: confirmed veclite-ffi is cdylib+staticlib+lib (phase4a); the committed golden header is phase4g scope, so phase4e generates veclite.h via cbindgen (cbindgen.toml added) for packaging; read SPEC-008 §FFI-006/007/030 + SPEC-016 §3 artifact matrix / §6 checklist
- [x] 1.2 crate-type + naming — cdylib+staticlib already set; shipped names (libveclite.{so,dylib}/veclite.dll/libveclite.a) via SONAME (RUSTFLAGS -soname libveclite.so), install_name (@rpath/libveclite.dylib), and on Windows rename + regenerate the import lib to reference veclite.dll (lib renamed → rlib collision, so file-rename is the correct path). Verified end-to-end locally on Windows.
- [x] 1.3 CI build matrix — veclite-release.yml `ffi-libs` job: glibc+musl x86_64/aarch64 (setup-cross-toolchain), macOS x86_64/aarch64 + `ffi-universal-macos` (lipo universal2), Windows x86_64/aarch64 (MSVC via msvc-dev-cmd)
- [x] 1.4 Package bundles — per-target veclite-ffi-<target>.tar.gz {shared, static, veclite.h, LICENSE, SHA256SUMS}; `publish-ffi-release` uploads to the GitHub release with a top-level SHA256SUMS.txt
- [x] 1.5 Per-platform C smoke-link — ffi/smoke.c + ffi/smoke.sh link the shipped lib (no Rust) and call vl_open_memory/vl_db_close/version; `ffi-smoke` job on Linux/macOS/Windows; green locally on Windows (via regenerated import lib)
- [x] 1.6 Docs — docs/c-abi.md: what ships, supported-target matrix, download+checksum verify, per-OS link instructions, minimal program

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 2.1 Update or create documentation covering the implementation — docs/c-abi.md + CHANGELOG entry + cbindgen.toml
- [x] 2.2 Write tests covering the new behavior — ffi/smoke.c + ffi/smoke.sh (per-platform smoke-link in the release workflow, REL-020 no-toolchain gate)
- [x] 2.3 Run tests and confirm they pass — smoke-link green locally on Windows; veclite-ffi unit tests (27) + workspace build green after the lib-name revert

