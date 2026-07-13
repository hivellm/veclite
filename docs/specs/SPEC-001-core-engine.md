# SPEC-001 — Core Engine

| | |
|---|---|
| **Status** | Draft (normative once Phase 1 starts) |
| **Phase / tasks** | Phase 0–1 · T0.2, T1.1–T1.5 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-10, FR-11, FR-14, FR-20–23, FR-30; NFR-01, NFR-04, NFR-07–10 |
| **Planning source** | [02-architecture.md](../vectorizer-lite/02-architecture.md), [01-vision-and-scope.md](../vectorizer-lite/01-vision-and-scope.md) |

Keywords **MUST**, **MUST NOT**, **SHOULD**, **MAY** are RFC 2119. Requirement IDs `CORE-xxx` are stable.

## 1. Scope

The in-memory engine of the `veclite` crate: collection registry, vector CRUD, HNSW indexing, quantization, distance computation, and the concurrency model. Persistence is SPEC-002/003; the public API surface is SPEC-004.

## 2. Dependencies and reuse

- **CORE-001** The crate MUST depend on the published `vectorizer-core` crate for: quantization (scalar/product/binary), SIMD distance kernels (AVX2/NEON/simd128 with runtime detection), LZ4/zstd compression, and codec primitives. These MUST NOT be reimplemented — bit-identical math is the parity guarantee (PRD G1, FR-73).
- **CORE-002** Engine logic (collection management, HNSW wrapper, payload index, hybrid search) is **extract-and-adapt** from the Vectorizer workspace (`db/` modules): copied into `veclite`, with async, server, tenant, GPU, and shard branches removed. Provenance comments MUST reference the source file and Vectorizer version.
- **CORE-003** `vectorizer-core` MUST be pinned to a minor line (initially `"3.5"`). Any change to quantization encodings, distance math, or HNSW serialization MUST land in `vectorizer-core` first (shared-crate policy, [07-compat §repo relationship](../vectorizer-lite/07-vectorizer-compatibility.md)).
- **CORE-004** The default build MUST compile on `x86_64/aarch64` Linux, macOS, Windows and `wasm32-unknown-unknown` (engine without file storage). Verified in CI from T0.2 onward.

## 3. Data model

### 3.1 Identifiers

- **CORE-010** Vector IDs are UTF-8 strings, 1–512 bytes. Empty IDs MUST be rejected (`InvalidArgument`). Numeric IDs from bindings pass through as their decimal string form.
- **CORE-011** Collection names: UTF-8, 1–255 bytes, MUST NOT contain `/`, `\`, NUL, or leading/trailing whitespace; unique per database. Aliases share the same namespace as names (a name lookup resolves aliases transparently).

### 3.2 Points

```rust
pub struct Point {
    pub id: String,
    pub vector: Vec<f32>,               // dense lane
    pub sparse: Option<SparseVector>,   // optional sparse lane (SPEC-007)
    pub payload: Option<serde_json::Value>,
}
pub struct SparseVector { pub indices: Vec<u32>, pub values: Vec<f32> }
pub struct Hit { pub id: String, pub score: f32,
                 pub payload: Option<serde_json::Value>, pub vector: Option<Vec<f32>> }
```

- **CORE-012** `upsert` of a vector whose length ≠ collection dimension MUST return `DimensionMismatch { expected, got }` and MUST NOT modify state. No silent truncation/padding, ever (FR-23).
- **CORE-013** Vectors containing NaN or ±Inf MUST be rejected with `InvalidArgument` at ingest.
- **CORE-014** For `Metric::Cosine`, vectors are normalized at ingest (stored normalized); the query vector is normalized at search time. Zero vectors under Cosine MUST be rejected (`InvalidArgument`).

### 3.3 Collection configuration

- **CORE-015** `CollectionConfig` fields and defaults (server parity — see SPEC-004 §3 defaults table): `dimension: usize (1..=65_536)`, `metric`, `hnsw: { m: 16, ef_construction: 200, ef_search: 100 }`, `quantization: Scalar { bits: 8 }`, `compression: Lz4 { threshold: 1024 }`, `embedding_provider: Option<String>`, `payload_indexes: Vec<(String, PayloadIndexKind)>`.
- **CORE-016** Config is immutable after creation except: `ef_search` default (mutable), payload indexes (addable), aliases. Changing dimension/metric/quantization requires a new collection.

## 4. Collection registry

- **CORE-020** The registry maps name → collection handle using a concurrent map (`DashMap`, as the server does). `create_collection` with an existing name (or alias) MUST return `AlreadyExists`.
- **CORE-021** `delete_collection` MUST drop all in-memory state and (when persistent) tombstone the collection's segments for the next checkpoint. Open `Collection` handles to a deleted collection MUST return `CollectionNotFound` on subsequent operations.
- **CORE-022** `rename_collection` and alias operations are metadata-only and MUST be O(1) in vector count.

## 5. HNSW index

- **CORE-030** The ANN index is `hnsw_rs 0.3`, wrapped by the adaptation of `db/optimized_hnsw.rs` (CPU path, synchronous). The exact `hnsw_rs` version MUST be pinned (`=0.3.x`) because graph serialization stability is not guaranteed upstream.
- **CORE-031** Parameters `m`, `ef_construction` are fixed per collection; `ef_search` has a collection default overridable per query. Bounds: `m ∈ 4..=64`, `ef_construction ∈ 8..=2048`, `ef_search ∈ 1..=4096`; out-of-range → `InvalidArgument`.
- **CORE-032** Deletes are **soft**: the node is added to the tombstone set and excluded from results at query time. `reindex()` rebuilds the graph from live vectors, purging tombstones. Search MUST over-fetch internally when tombstones are present so that `limit` live results are returned whenever the collection holds ≥ limit live vectors.
- **CORE-033** Upsert of an existing ID replaces the vector: implemented as soft-delete + insert (graph nodes are not mutated in place).
- **CORE-034** Batch inserts MAY parallelize HNSW construction with `rayon` scoped threads. Insertion order MUST NOT affect result correctness (recall targets), only graph shape.
- **CORE-035** Search MUST return results ordered by score: descending for Cosine/DotProduct similarity, ascending for Euclidean distance. Score semantics match the server exactly (parity harness T1.7 is the arbiter).

## 6. Quantization

- **CORE-040** Default quantization is `Scalar { bits: 8 }` (SQ-8), matching the server default. `Quantization::None`, `Scalar { bits: 4|2|1 }`, `Binary`, and (feature `pq`) `Product` MUST be selectable per collection.
- **CORE-041** Encodings come from `vectorizer-core` and MUST be byte-compatible with the server's (`.vecdb` interop, SPEC-013, depends on this).
- **CORE-042** SQ scale/offset parameters are computed per segment (persistent) or per collection (in-memory) and stored alongside the codes. Re-quantization on `vacuum`/`reindex` MAY recompute them.
- **CORE-043** Search on quantized collections scores against quantized codes using the matching `vectorizer-core` kernel. SQ-8 recall vs unquantized MUST be ≥ 0.99 top-10 on the standard corpus (T1.3 exit test).

## 7. Concurrency model

- **CORE-050** `Database` and `Collection` handles are `Send + Sync + Clone` (internal `Arc`). Multiple `Database` instances in one process MUST be fully independent — no global state, no singletons.
- **CORE-051** Locking (server-parity primitives): registry = `DashMap`; per-collection = `parking_lot::RwLock`. Reads (`search`/`get`/`scroll`/`stats`) take the read lock and run concurrently. Writes take the write lock; writes to a collection are additionally serialized by the WAL appender (SPEC-003).
- **CORE-052** The engine MUST NOT spawn background threads unless `OpenOptions::background_checkpoint(true)` (NFR-07). Internal `rayon` use is scoped (join-before-return) and MUST be disabled on wasm32.
- **CORE-053** No operation may hold a lock across user-visible blocking I/O except the short TOC-swap window defined in SPEC-002 §6.
- **CORE-054** The concurrent read/write soak (T6.2) MUST pass under ThreadSanitizer; targeted `loom` tests SHOULD cover the checkpoint/reader interleavings.

## 8. Error policy

- **CORE-060** All fallible operations return `Result<_, VecLiteError>` (single `thiserror` enum, SPEC-004 §6). Workspace lints: `unwrap_used = "deny"`, `expect_used = "deny"` in library code (NFR-09).
- **CORE-061** Panics indicate broken internal invariants only; they MUST NOT cross the public Rust API in safe usage and are caught at the FFI boundary (SPEC-008).

## 9. Anti-requirements

- **CORE-070** Nothing in core performs network I/O (NFR-08). Enforced by a CI deny-list on the dependency tree of the default build (no `reqwest`/`hyper`/`tokio` etc.).
- **CORE-071** No plugin system or dynamic provider loading; embedding providers are compile-time features + the in-process `register_embedder` hook (SPEC-005).
- **CORE-072** No async functions in the core crate.

## 10. Acceptance criteria

1. CRUD property tests (T1.1): arbitrary op sequences vs a model `HashMap` — state equivalence.
2. Recall harness (T1.2/T1.3): HNSW top-10 recall ≥ 0.95 vs brute force at defaults; SQ-8 ≥ 0.99 vs unquantized.
3. Parity (T1.7): same corpus into VecLite and Vectorizer server → top-10 overlap ≥ 0.99 (NFR-04).
4. Perf (T1.6): 1 M × 512-dim, p50 < 3 ms search on the reference profile (NFR-01).
5. Cross-target CI matrix green incl. wasm32 skeleton (CORE-004).
