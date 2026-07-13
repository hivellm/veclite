# VecLite — Specifications

This directory is the **implementation contract** for VecLite, derived from the planning set in [`docs/vectorizer-lite/`](../vectorizer-lite/README.md). The planning docs explain *why* and map code back to the Vectorizer source; these documents say *what to build*, normatively.

## How to navigate

| Question | Read |
|---|---|
| What are we building and why? What are the requirements and release criteria? | [**PRD.md**](../PRD.md) |
| What order does the work happen in? What blocks what? | [**DAG.md**](../DAG.md) |
| How exactly does component X behave? | The SPEC for that component (below) |
| Why was it designed this way? | [`docs/vectorizer-lite/`](../vectorizer-lite/README.md) |

Traceability chain: **PRD** requirement IDs (`FR-xx`, `NFR-xx`) → **DAG** tasks (`T<phase>.<n>`) → **SPEC** requirement IDs (`CORE-xxx`, `STG-xxx`, …) → tests (SPEC-015).

## Specifications

| Spec | Scope | Freeze event |
|---|---|---|
| [SPEC-001](SPEC-001-core-engine.md) — Core Engine | Collections, CRUD, HNSW, quantization, concurrency model, reuse of `vectorizer-core` | API freeze (T4.1) |
| [SPEC-002](SPEC-002-storage-format.md) — Storage Format | `.veclite` v1 file layout: header, segments, TOC, commit protocol, snapshot/vacuum, limits | **Format freeze (G2)** |
| [SPEC-003](SPEC-003-wal-durability.md) — WAL & Durability | WAL entries, durability modes, checkpoint, recovery, close semantics | Format freeze (G2) |
| [SPEC-004](SPEC-004-rust-api.md) — Rust API | Public API surface (source of truth for all bindings), defaults, errors, feature flags, evolution rules | API freeze (T4.1) |
| [SPEC-005](SPEC-005-embeddings.md) — Embeddings | Provider tiers/matrix, `Embedder` trait, auto-embed collections, vocabulary lifecycle, chunker, `onnx` | — |
| [SPEC-006](SPEC-006-payload-filters.md) — Payloads & Filters | Payload storage, payload indexes, filter model and execution | — |
| [SPEC-007](SPEC-007-hybrid-search.md) — Hybrid Search | Sparse vectors, postings, RRF fusion semantics | — |
| [SPEC-008](SPEC-008-ffi-c-abi.md) — FFI / C ABI | `veclite-ffi` contract for Go/C#/community bindings; error-code table | ABI freeze (1.0.0) |
| [SPEC-009](SPEC-009-binding-python.md) — Python Binding | PyO3, abi3 wheels, NumPy zero-copy, GIL, asyncio facade | — |
| [SPEC-010](SPEC-010-binding-node.md) — Node.js Binding | napi-rs, async + sync twins, Float32Array zero-copy, prebuilds | — |
| [SPEC-011](SPEC-011-bindings-go-csharp.md) — Go & C# Bindings | cgo and P/Invoke over the C ABI | — |
| [SPEC-012](SPEC-012-binding-wasm.md) — WASM Package | Browser/edge profile: memory/OPFS/serialize backends, budgets | — |
| [SPEC-013](SPEC-013-vecdb-interop.md) — Vectorizer Interop | `.vecdb` import/export, graduation path, divergence policy | — |
| [SPEC-014](SPEC-014-cli.md) — CLI | `veclite` binary: inspect/export/import/vacuum/snapshot/verify | — |
| [SPEC-015](SPEC-015-testing-conformance.md) — Testing & Conformance | Crash suites, binding conformance corpus, parity harness, benchmarks, fuzzing | — |
| [SPEC-016](SPEC-016-packaging-release.md) — Packaging & Release | CI matrix, artifact matrix, versioning policy, 1.0 checklist | — |

## Conventions

- RFC 2119 keywords (**MUST**, **MUST NOT**, **SHOULD**, **MAY**) are normative.
- Requirement IDs are stable and referenced from commits, PRs, and tests. Removing or changing the meaning of an ID requires the same review bar as the behavior change itself.
- Two hard freezes exist: the **storage format** freezes at gate G2 (after that, changes bump the format version) and the **public API** freezes at task T4.1 (after that, additive-only within 1.x).
- Resolved [PRD open questions](../PRD.md#12-open-questions) so far: **OQ-2** → SPEC-016 REL-002 (MSRV tracks `vectorizer-core`) · **OQ-3** → SPEC-012 WASM-011 (full-image OPFS buffering) · **OQ-4** → SPEC-014 header (separate `veclite-cli` crate) · **OQ-5** → SPEC-002 §3.1 (MessagePack everywhere). **OQ-1** (reference benchmark hardware) remains open until T1.6.

## Change control

1. A change to product behavior lands as: PRD update (if requirements change) + SPEC update + DAG update (if work items change) — in the same PR.
2. Post-freeze changes to SPEC-002/003 require a format-version bump proposal; post-freeze changes to SPEC-004/008 must be additive (see each spec's evolution rules).
3. Divergence between these specs and the planning docs: the **specs win**; update the planning doc or record the decision here.
