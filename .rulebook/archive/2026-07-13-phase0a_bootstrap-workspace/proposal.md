# Proposal: phase0a_bootstrap-workspace

## Why
VecLite has no code yet. DAG tasks T0.1–T0.3 bootstrap the cargo workspace, CI pipeline, and base types so every later context cycle compiles against pinned dependencies and server-parity defaults. Gate G0 blocks all downstream work.

> **Addendum (ADR-0001, during execution):** the originally planned `vectorizer-core = "3.5"` dependency was removed by user decision — VecLite has zero dependencies on Vectorizer crates; needed code is vendored copy-on-need (quantization/SIMD → phase1b, compression → phase2a). Items 1.3/1.4 were re-scoped accordingly.

## What Changes
- Cargo workspace with `crates/veclite` lib skeleton; own `rust-version` pin (REL-002, 1.85)
- Feature flags `default = ["simd"]` (API-050); no Vectorizer dependency (ADR-0001)
- Port `VecLiteError`, `Metric`, `CollectionOptions`/`OpenOptions` with server-parity defaults (SPEC-004 §3/§6)
- CI: fmt + clippy -D warnings (unwrap_used deny) + tests on Linux/macOS/Windows + wasm32 build check + network-crate deny-list (REL-010, NFR-08)

## Impact
- Affected specs: SPEC-001 (CORE-001..004), SPEC-004 (API-040/050), SPEC-016 (REL-001/002/010)
- Affected code: Cargo.toml, crates/veclite/src/{lib,error,options}.rs, .github/workflows/
- Breaking change: NO
- User benefit: reproducible foundation; G0 exit — cargo test green on 3 OS, wasm32 compiles
