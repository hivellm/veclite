## 1. Implementation
- [ ] 1.1 Context: confirm phase4a `veclite-ffi` (cdylib+staticlib) + golden header exist; read SPEC-008, SPEC-016 release checklist
- [ ] 1.2 crate-type + naming: cdylib + staticlib in veclite-ffi; set SONAME (Linux), install_name (macOS), and DLL base name
- [ ] 1.3 CI build matrix: Linux glibc x86_64/aarch64, Linux musl x86_64/aarch64, macOS x86_64/aarch64 (+ universal2), Windows x86_64/aarch64 (MSVC) — cross-compile where no native runner
- [ ] 1.4 Package per-platform bundle: shared lib + static lib + veclite.h + LICENSE + SHA256SUMS; upload to GitHub release
- [ ] 1.5 Per-platform C smoke-link test (ffi/smoke.c): link the artifact, call vl_open/vl_close, run with no Rust toolchain present
- [ ] 1.6 Docs: per-OS download/link instructions + supported-target matrix on the docs site

## 2. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 2.1 Update or create documentation covering the implementation
- [ ] 2.2 Write tests covering the new behavior (smoke-link per platform in CI)
- [ ] 2.3 Run tests and confirm they pass

