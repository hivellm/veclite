# Proposal: phase4e_ffi-shared-lib-artifacts

## Why
phase4a builds the `veclite-ffi` crate (cdylib + staticlib) and its cbindgen
header, and phase4d packages the **language bindings** (Python wheels via
maturin, Node prebuilds via napi). Neither ships the **raw C ABI shared
libraries** — `veclite.dll` / `libveclite.so` / `libveclite.dylib` (plus the
static libs and the header) — as downloadable, checksummed release artifacts
for every OS and architecture. Direct C consumers, and the Go and C# bindings
(phase5a) that load the C ABI, need prebuilt libraries on every supported
platform; today that cross-OS build+release matrix is not a planned deliverable.
This task closes that gap.

## What Changes
- CI release matrix that builds `veclite-ffi` (cdylib + staticlib) on every
  supported target: Linux glibc x86_64/aarch64, Linux musl x86_64/aarch64,
  macOS x86_64/aarch64 (plus a universal2 `.dylib`), Windows x86_64/aarch64
  (MSVC). Cross-compile via `cross`/target toolchains where no native runner
  exists.
- Package each platform bundle: shared library, static library, committed
  golden `veclite.h`, LICENSE, and a `SHA256SUMS`; upload to the GitHub release.
- Per-platform smoke-link test: a tiny C program links the artifact and calls
  `vl_open`/`vl_close`, proving the shipped library loads with no Rust toolchain
  present (mirrors phase4d's clean-machine bar).
- Configure `SONAME`/`install_name`/DLL naming and document per-OS
  download/link instructions and the supported-target matrix.
- Out of scope: the `wasm32`/`.wasm` package (that is phase5b, OPFS) — this task
  is native shared libraries only.

## Impact
- Affected specs: SPEC-008 (C ABI) distribution surface; SPEC-016 release
  checklist gains the shared-library artifacts. No format or API change.
- Affected code: `.github/workflows/release.yml` (ffi artifact matrix), a small
  `ffi/smoke.c` link test, `crates/veclite-ffi/` build config, docs.
- Breaking change: NO
- User benefit: C/Go/C#/direct integrators get official, checksummed prebuilt
  libraries for every OS+arch instead of building the FFI crate themselves;
  unblocks the phase5a Go/C# bindings. Depends on phase4a; complements phase4d.
