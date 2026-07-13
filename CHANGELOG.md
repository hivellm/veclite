# Changelog

All notable changes to VecLite are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Versions 0.x are pre-release: the public API may change between minors until 1.0.0.

## [Unreleased]

### Added
- Cargo workspace bootstrap (`crates/veclite`), edition 2024, provisional MSRV 1.85,
  workspace lints denying `unwrap`/`expect` in library code (task `phase0a`, DAG T0.1/T0.3).
- `VecLiteError` with the full stable variant set and pinned display strings (SPEC-004 §6).
- `CollectionOptions`, `OpenOptions`, `Metric`, `Quantization`, `Compression`,
  `Durability`, `PayloadIndexKind`, `HnswOptions` with server-parity defaults
  (SPEC-004 §3), covered by defaults-table unit tests.
- CI: wasm32-unknown-unknown build check, network-crate deny-list (NFR-08),
  and MSRV build job (`veclite-checks.yml`), alongside the existing
  3-OS test and lint workflows.
- `.editorconfig` (4-space indentation) and Rust entries in `.gitignore`.

### Changed
- **ADR-0001**: VecLite has zero dependency on Vectorizer crates. The originally
  planned `vectorizer-core` dependency (unpublished; mandatory network deps conflict
  with NFR-08) is replaced by a vendoring policy — needed code is copied into this
  repo with provenance headers, byte-identical encodings enforced by the conformance
  corpus. Quantization/SIMD land with `phase1b`, compression with `phase2a`.
