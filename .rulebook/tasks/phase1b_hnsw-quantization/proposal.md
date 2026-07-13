# Proposal: phase1b_hnsw-quantization

## Why
DAG T1.2 + T1.3: k-NN search requires the HNSW index extracted from Vectorizer's optimized_hnsw (sync CPU path) and the SQ-8 quantization default that defines VecLite's memory profile and server parity (FR-11, FR-14, FR-30).

## What Changes
- HNSW wrapper over pinned hnsw_rs =0.3.x: insert, search, soft-delete tombstones, reindex() (CORE-030..035)
- Parameter bounds validation (m 4..=64, ef_construction 8..=2048, ef_search 1..=4096)
- Quantization + SIMD kernels vendored from the Vectorizer repo (ADR-0001, byte-identical encodings); SQ-8 on ingest/search; Quantization::None/Binary selectable (CORE-040..043)
- rayon-parallel batch inserts (scoped, disabled on wasm32)

## Impact
- Affected specs: SPEC-001 §5–6
- Affected code: crates/veclite/src/index/, quantization glue in collection.rs
- Breaking change: NO
- User benefit: real ANN search with 4x memory savings by default; recall gates guarantee quality
