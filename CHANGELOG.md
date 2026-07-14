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
- In-memory engine `VecLite::memory()`: collection registry over `DashMap`
  (`create_collection`/`collection`/`delete_collection`/`rename_collection`/
  `list_collections` with `AlreadyExists`/`CollectionNotFound` semantics and
  stale-handle guards) and vector CRUD (`upsert`/`upsert_batch`/`get`/`delete`/
  `delete_batch`/`len`) with dimension and NaN/Inf rejection and cosine ingest
  normalization; `Send + Sync + Clone` handles (task `phase1a`, DAG T1.1/T1.5,
  SPEC-001 §3–4, SPEC-004 §1/§4, CORE-010..014/020..022/050/051).
- `Point`, `SparseVector`, and `Hit` data-model types with id and
  collection-name validation (CORE-010/011).
- HNSW index over the pinned `hnsw_rs =0.3.4`, adapted from the server's
  `optimized_hnsw` and generalized to Cosine/Euclidean with static enum
  dispatch, per-collection `m`/`ef_construction` bounds (CORE-031),
  soft-delete tombstones + search-time over-fetch (CORE-032/033), `reindex()`,
  and rayon batch insert. Native-only — target-gated off wasm32 (ADR-0002,
  task `phase1b`, DAG T1.2/T1.3, SPEC-001 §5).
- Vendored quantization (SQ-8 default, scalar 4/2/1-bit, binary; product
  behind the `pq` feature) and scalar SIMD distance/quantize kernels, both
  byte-identical to `vectorizer-core@3.5.0` (CORE-040..043/001). Fixes the
  upstream SQ `deserialize_params` offset-restore bug without changing the
  serialized shape. Recall gates pass: HNSW top-10 ≥ 0.95, SQ-8 vs
  unquantized ≥ 0.99.
- Public search API (task `phase1c`, DAG T1.4, SPEC-004 §4–5): `Collection::search(vector, limit)`
  and the `query()` builder (`limit` default 10, per-query `ef_search`,
  `with_payload` default true, `with_vector` default false, and a declared
  `filter` slot for phase3a). Results are ordered per metric (CORE-035):
  descending similarity for Cosine/DotProduct, ascending distance for
  Euclidean. Cosine/Euclidean use the HNSW index; DotProduct and any metric on
  wasm32 use exact brute force. Builders hold no lock until `run()` (API-030);
  `limit = 0` → `InvalidArgument`, `limit` above the live count returns all
  live (API-031).
- Benchmark + server-parity gate G1 (task `phase1d`, DAG T1.6/T1.7, SPEC-015 §4–5):
  criterion benches (`benches/veclite_bench.rs`: search p50 ≈ 0.92 ms at
  2k×512, index build, batch insert), a server-parity harness
  (`tests/parity.rs`) that loads an identical corpus into VecLite and a pinned
  `hivehub/vectorizer:3.5.0` container and asserts **top-10 overlap ≥ 0.99**
  (measured 0.9920), a CI smoke-bench workflow with a ±20 % regression fence
  plus nightly full bench, and the reference hardware profile in
  [docs/benchmarks.md](docs/benchmarks.md). The harness is gated on
  `VECLITE_PARITY_URL` (no server needed for a normal `cargo test`) and uses a
  dependency-free HTTP client so no network crate enters the shipped build
  (NFR-08).

- `.veclite` on-disk format v1 storage layer (task `phase2a`, DAG T2.1/T2.2,
  SPEC-002 §1–5, native-only — gated off wasm32 per CORE-004): the fixed 4 KiB
  header with `header_crc32` and `min_reader_version` gate; the immutable
  segment codec (32-byte header, per-segment crc32 naming `segment@<offset>` on
  mismatch, LZ4/zstd bodies vendored for `.vecdb` byte-compat, VECTORS never
  compressed); all nine segment bodies (CONFIG/PAYLOAD/PIDX/CONFIG via
  MessagePack, TOMBSTONE/PIDX via 64-bit roaring, VECTORS fixed-stride with
  mmap slot addressing, IDDIR xxhash64 hash-bucketed directory, SPARSE, VOCAB,
  HNSW); the MessagePack TOC with a generation counter and deterministic replay
  order (STG-041); and the root-pointer-swap commit protocol —
  segments→fsync→TOC→fsync→header→fsync (STG-050). Property round-trips, decode
  fuzz (arbitrary bytes never panic), and a commit-crash-sequence test (a torn
  tail beyond the committed header leaves the previous TOC valid) all pass.
- Durable single-file database: `VecLite::open(path)` / `open_with(path, opts)`
  and `db.checkpoint()` (task `phase2b`, DAG T2.3/T2.4, SPEC-003, native-only).
  The `<db>.veclite-wal` sidecar logs every mutation (8 op types, MessagePack
  bodies) with a uuid-prefix stale-sidecar guard; the three `Durability` modes
  wire into every write (`Full` fsyncs each append). A checkpoint seals the
  live state into segments via the commit protocol then truncates the WAL after
  the header-swap fsync (WAL-031/032); it triggers on the WAL size limit,
  `checkpoint()`, or last-handle drop. Recovery replays a non-clean WAL in seq
  order, discarding a torn tail and applying entries idempotently
  (WAL-040..042). Verified end-to-end: checkpoint→reopen with rebuilt HNSW
  search, crash→WAL-replay equals a model over 200 random ops with interleaved
  checkpoints, delete/rename durability, torn-tail recovery, and the stale-WAL
  guard.

### Changed
- **PRD OQ-1 resolved** (phase1d): the reference hardware profile is pinned in
  [docs/benchmarks.md](docs/benchmarks.md) — a desktop AMD Ryzen 9 7950X3D class
  and an AWS `c7a.4xlarge` cloud class.
- **ADR-0001**: VecLite has zero dependency on Vectorizer crates. The originally
  planned `vectorizer-core` dependency (unpublished; mandatory network deps conflict
  with NFR-08) is replaced by a vendoring policy — needed code is copied into this
  repo with provenance headers, byte-identical encodings enforced by the conformance
  corpus. Quantization/SIMD land with `phase1b`, compression with `phase2a`.
