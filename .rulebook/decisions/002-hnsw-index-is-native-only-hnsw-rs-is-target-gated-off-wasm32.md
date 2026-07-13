# 2. HNSW index is native-only; hnsw_rs is target-gated off wasm32

**Status**: proposed
**Date**: 2026-07-13
**Related Tasks**: phase1b_hnsw-quantization, phase1c_search-query-api, phase5b_wasm-package

## Context

phase1b pins the ANN index to hnsw_rs =0.3.x (SPEC-001 CORE-030), but hnsw_rs 0.3.4 cannot compile for wasm32-unknown-unknown: it unconditionally pulls cpu-time (libc), rayon (threads), and mmap-rs, none of which build on wasm. Meanwhile CORE-004 and the veclite-checks.yml CI gate require the default build to compile on wasm32-unknown-unknown. As a normal dependency, hnsw_rs makes the wasm gate impossible. Quantization and SIMD, by contrast, are pure-Rust std::arch with an always-present scalar fallback and vendor cleanly to every target including wasm.

## Decision

hnsw_rs and the HNSW index module are native-only, declared under [target.'cfg(not(target_arch = \"wasm32\"))'.dependencies] and gated with #[cfg(not(target_arch = \"wasm32\"))]. rayon (batch insert) is gated the same way (CORE-052). On wasm32 the crate compiles the engine — collection registry, vector CRUD, quantization, and SIMD distance kernels — but ships no ANN index; this matches CORE-004's scoping of wasm to the engine without file storage. Quantization and SIMD remain unconditional and compile on all targets. The wasm search-time behavior (return an error vs. brute-force exact search) is deferred to phase1c when the public search() API lands.

## Alternatives Considered

- Depend on hnsw_rs normally (impossible: breaks the wasm32 CI gate, CORE-004)
- Add a scalar brute-force search fallback on wasm now (deferred to phase1c with the search API; out of phase1b scope)
- Fork hnsw_rs 0.3.4 into VecLite and strip cpu-time/rayon/mmap-rs (large effort, high maintenance, risks diverging from the pinned upstream graph-serialization format)

## Consequences

Pros: the wasm32 build stays green with no fork; native targets get the full pinned hnsw_rs index; quantization/SIMD are shared across all targets. Cons: wasm32 has no ANN index, so search on wasm needs a separate decision in phase1c; any code touching the index must respect the cfg gate, and tests exercising the index are native-only. Refs SPEC-001 CORE-004/030/052; supersedes the naive reading of CORE-030 that assumed a normal dependency.
