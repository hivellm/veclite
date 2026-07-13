# Proposal: phase0a_bootstrap-workspace

## Why
VecLite has no code yet. DAG tasks T0.1–T0.3 bootstrap the cargo workspace, CI pipeline, and base types so every later context cycle compiles against pinned dependencies and server-parity defaults. Gate G0 blocks all downstream work.

## What Changes
- Cargo workspace with `crates/veclite` lib skeleton; `rust-version` pinned to vectorizer-core MSRV (REL-002)
- Dependency `vectorizer-core = "3.5"`; feature flags `default = ["simd"]` (API-050)
- Port `VecLiteError`, `Metric`, `CollectionOptions`/`OpenOptions` with server-parity defaults (SPEC-004 §3/§6)
- CI: fmt + clippy -D warnings (unwrap_used deny) + tests on Linux/macOS/Windows + wasm32 build check + network-crate deny-list (REL-010, NFR-08)

## Impact
- Affected specs: SPEC-001 (CORE-001..004), SPEC-004 (API-040/050), SPEC-016 (REL-001/002/010)
- Affected code: Cargo.toml, crates/veclite/src/{lib,error,options}.rs, .github/workflows/
- Breaking change: NO
- User benefit: reproducible foundation; G0 exit — cargo test green on 3 OS, wasm32 compiles
