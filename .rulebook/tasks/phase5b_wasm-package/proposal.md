# Proposal: phase5b_wasm-package

## Why
DAG T5.3: WASM is where a server can never go — browsers, extensions, edge runtimes. PRD OQ-3 is already resolved (full-image OPFS buffering); this cycle ships @veclite/wasm within the 3 MB budget (FR-64).

## What Changes
- crates/veclite-wasm: wasm-bindgen build excluding file storage/mmap/locks/rayon/onnx (WASM-001)
- simd128 kernels + non-SIMD fallback with loader feature detection (WASM-002)
- Storage backends: in-memory, serialize()/deserialize() as valid .veclite v1 images, OPFS with save() + autosave (WASM-010/011)
- API mirroring SPEC-010 minus vacuum/paths/locks; all methods async (WASM-020/021)
- Bundle budget <= 3 MB gzipped enforced in CI (WASM-030)

## Impact
- Affected specs: SPEC-012 (all)
- Affected code: crates/veclite-wasm/ (new), @veclite/wasm npm package
- Breaking change: NO
- User benefit: client-side semantic search over ~500k vectors, offline, no server — with files interchangeable with native VecLite
