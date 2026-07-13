# 1. Vendor engine code from Vectorizer instead of depending on vectorizer-core

**Status**: proposed
**Date**: 2026-07-13
**Related Tasks**: phase0a_bootstrap-workspace, phase1b_hnsw-quantization, phase2a_pager-segments

## Context

The planning set (docs/vectorizer-lite/02-architecture.md) and SPEC-001 CORE-001..003 mandated depending on the published vectorizer-core crate for quantization, SIMD kernels, and compression. Research during phase0a showed this is impossible: vectorizer-core 3.5.0 is not published on crates.io, and its mandatory dependencies (axum, tonic, rmcp, umicp-core with http2/websocket) violate VecLite's NFR-08 (no network crates in the default build). The user then set a hard project constraint: VecLite must have NO dependency on the Vectorizer project — anything needed gets copied into this repo.

## Decision

VecLite vendors (copy-and-adapt) the code it needs from hivellm/vectorizer into crates/veclite, with provenance headers naming the source file and commit. Copy-on-need per context cycle: quantization + SIMD distance kernels land with phase1b (their first consumer), compression with phase2a. Encodings and math MUST remain byte-identical to the server's; parity is enforced by the shared conformance corpus and the parity harness (NFR-04) instead of a shared crate. No path, git, or crates.io dependency on any Vectorizer crate, ever.

## Alternatives Considered

- Depend on published vectorizer-core from crates.io (impossible: unpublished; mandatory network deps violate NFR-08)
- Git dependency on hivellm/vectorizer pinned to a rev (rejected by user: no dependency on Vectorizer at all)
- Upstream PR feature-gating vectorizer-core wire-error mappings, then depend (rejected by user for the same reason)
- Relax NFR-08 (rejected: defeats the embedded product thesis)

## Consequences

Pros: zero coupling to the server's release train; default build stays pure-Rust and lean; wasm32 unblocked. Cons: bug fixes in shared math no longer flow automatically — any change to quantization encodings, distance math, or HNSW serialization must be manually ported and re-verified against the conformance corpus in both repos (parity risk shifts from crate versioning to test discipline). SPEC-001 §2, SPEC-013 §1, SPEC-016 REL-003/REL-033, PRD G1, and DAG T0.2 are updated to the vendoring policy.
