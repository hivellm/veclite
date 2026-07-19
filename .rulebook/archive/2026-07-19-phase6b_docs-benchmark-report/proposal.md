# Proposal: phase6b_docs-benchmark-report

## Why
DAG T6.3 + T6.4: 1.0 requires the docs site with CI-executed quickstarts for all 6 languages, both-direction migration guides, and the honest reproducible benchmark report against peer embedded stores (PRD §8, §9.8, REL-040/041).

## What Changes
- Docs site: quickstarts (Rust/Python/Node/Go/C#/WASM) each extracted and executed in CI; API reference per language; storage-format doc (frozen SPEC-002); sizing/limits page; WASM sizing guidance (REL-040)
- Migration guides: graduation (VecLite → server) and reverse (server → VecLite)
- Benchmark report: VecLite vs sqlite-vec, LanceDB embedded, Chroma embedded, Vectorizer server — datasets, code, hardware disclosed; losses published too (TST-042)
- Sample-runner CI: stale doc samples fail the build (REL-041)

## Impact
- Affected specs: SPEC-016 §5, SPEC-015 §5
- Affected code: docs site sources, tests/bench/report harness, CI sample-runner
- Breaking change: NO
- User benefit: credible, verifiable public story — every code sample provably runs, every benchmark reproducible
