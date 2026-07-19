# Proposal: phase6a_fuzz-soak-hardening

## Why
DAG T6.1 + T6.2: 1.0 hardening — adversarial inputs (fuzzing), sustained load (24 h soak), memory pressure (4x RAM mmap), and data-race verification (sanitizers, loom) must all be clean before the release cycle starts (NFR-05, NFR-10).

## What Changes
- cargo-fuzz targets: file/header/TOC/segment parser, WAL replay, filter document parser, MessagePack option decoding; corpus committed for regression (TST-050)
- 24 h soak: continuous write/search/vacuum/snapshot loop with invariant checks; RSS-plateau leak detection (TST-051)
- Memory-pressure runs: mmap datasets 4x RAM
- ASan + TSan integration-suite runs; targeted loom models for checkpoint/reader TOC-swap interleavings (TST-052, CORE-054)

## Impact
- Affected specs: SPEC-015 §6
- Affected code: fuzz/ (new), tests/soak/, CI scheduled jobs
- Breaking change: NO
- User benefit: 1.0 ships with 72 h fuzz-clean parsers and proven leak-free sustained operation
