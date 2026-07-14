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
- Single-writer safety and read-only open (task `phase2c`, DAG T2.6/T2.9,
  SPEC-002 §6, native-only). Open takes an advisory lock on the database file's
  own handle — exclusive for read-write, shared for read-only (`fs4`) — so a
  second opener of a locked file fails immediately with `Locked` instead of
  blocking or corrupting (STG-060). `OpenOptions::read_only(true)` serves reads
  and searches but rejects every mutation with `ReadOnly`, and refuses to open
  over a pending (uncheckpointed) WAL with `WalPending` unless
  `read_only_ignore_wal(true)` opts into reading the last checkpoint (STG-062,
  WAL-043). Both open modes tolerate a damaged tail beyond the committed TOC,
  reading the last committed state (STG-003). Covered by four integration
  tests (lock conflict, read-only read/write matrix, WalPending guard,
  damaged-tail in both modes).
- Snapshot, vacuum, and auto-vacuum (task `phase2d`, DAG T2.7/T2.8,
  SPEC-002 §7, native-only). `db.snapshot(path)` writes a standalone compacted
  copy with a fresh `file_uuid` — dead space and tombstones dropped — from the
  in-memory live state, so a concurrent writer keeps going without failure
  (STG-070); it works for in-memory databases too. `db.vacuum()` reclaims dead
  space: it compacts every collection in memory (dropping tombstoned slots and
  renumbering to `0..live`), rewrites the file to a single compacted generation
  with the **same** `file_uuid`, and shrinks it via a crash-safe
  close→rename→reopen swap (STG-071). A checkpoint escalates to a vacuum once a
  collection's tombstone ratio crosses `OpenOptions::auto_vacuum_threshold`
  (default 0.25, STG-072). Verified: snapshot-under-write standalone
  consistency, file shrink after a 50 % delete with the pager still live for
  further writes, and the auto-vacuum escalation threshold.
- Crash-safety gate G2 and format v1 freeze (task `phase2e`, DAG T2.10,
  SPEC-015 §2, NFR-05/NFR-11). A crash suite (`crates/veclite/tests/crash_safety.rs`)
  runs randomized upsert/delete/checkpoint workloads in all three durability
  modes with an oracle model (TST-010), a torn-WAL-tail sweep and a WAL bit-flip
  sweep (TST-011/012, WAL-011), a torn main-file tail check (STG-003), and
  whole-file bit-flip drills (TST-012) — every reopen matches the model or fails
  cleanly with `Corrupt`, never a panic or a silently wrong answer. Iteration
  count scales via `VECLITE_CRASH_ITERS`. `cargo xtask crash` (new `xtask` crate,
  SPEC-015 §7) runs the suite at 10 000 iterations plus a **real subprocess
  kill-9 harness** that SIGKILL/TerminateProcess-es a live `Full`-durability
  writer at random points and verifies every acked commit survives the reopen.
  The suite runs nightly on Linux/macOS/Windows (`veclite-crash.yml`, TST-013).
  Committed v1 golden files (`crates/veclite/tests/compat/golden/`) are guarded
  on every run (`tests/golden.rs`), and **SPEC-002/SPEC-003 are now
  frozen-normative**: the on-disk byte format is fixed; changes require a new
  format version (NFR-11).
- Payload filters (task `phase3a`, DAG T3.1–T3.3, SPEC-006). A Qdrant-style
  filter model — `Filter { must, should, must_not }` over `Condition::{Eq, In,
  Range, Exists, Nested}` with `MatchValue`/`Range` — with server-parity
  combination and type semantics (FLT-010/011): `must` AND, `should` OR (when
  non-empty), `must_not` NAND; integer/float JSON-number equality; `Range` on
  numerics only; `Exists` = key presence (matches `null`). Filters are built in
  Rust or parsed from a portable JSON document (`Filter::from_json`); geo
  conditions and nested-path keys are rejected with `InvalidArgument`, never
  ignored (FLT-012). `Collection::query(v).filter(f)` applies them. Payload
  indexes (`Keyword`/`Integer`/`Float`) declared via `CollectionOptions::
  payload_index` build roaring-bitmap `value → slots` maps that pre-filter
  selective queries; they are accelerators only — results are identical to a
  payload scan (FLT-022) and the index rebuilds from payloads on open. Top-level
  `_`-prefixed payload keys are reserved (FLT-002) and payloads over 16 MiB are
  rejected. Covered by a conformance corpus, index/scan-equivalence, pre-filter
  vs brute-force, reserved-key, and unsupported-feature tests (`tests/filters.rs`,
  gate G3 criteria 1–5).
- Text embeddings & auto-embed collections (task `phase3b`, DAG T3.5/T3.6,
  SPEC-005). An `Embedder` trait and four pure-Rust sparse providers vendored
  from the server (ADR-0001) with identical scoring math for parity (EMB-002):
  `bm25` (default; `k1=1.5`, `b=0.75`), `tfidf`, `bow`, and `char_ngram`
  (typo-tolerant trigrams); `veclite::build_provider(name, dim)` fails with
  `UnsupportedProvider` on an unknown name (EMB-021). `CollectionOptions::
  auto_embed(provider, dim)` collections accept `upsert_text`/`search_text`:
  the text is embedded and stored under the reserved `_text` key (EMB-022), and
  the vocabulary is a function of the live `_text` corpus — recomputed lazily
  before search and rebuilt from `_text` on reopen (like the HNSW graph), so
  `search_text` is reopen-identical (EMB-020) with no VOCAB segment yet. Text
  ops on a BYO collection and reserved user keys are rejected. A deterministic,
  UTF-8-safe `veclite::chunk::Chunker` (word/sentence boundaries, overlap;
  EMB-050/051) rounds it out. Covered by provider unit tests, a chunker UTF-8
  fuzz, and `tests/auto_embed.rs`.

### Fixed
- **Small-collection search recall** (phase3a): searches now return exact,
  correctly ordered results when the live set is no larger than the requested
  count, and fall back to exact brute force whenever the HNSW index
  under-returns — `search` always yields `min(limit, live)` results (previously
  a tiny/approximate graph could drop the farthest candidate).
- **WAL entry integrity** (phase2e): the per-entry CRC now covers the fixed
  header fields (`seq`/`coll_id`/`op`/`body_len`) in addition to the body, so a
  bit flip in a header field is detected and terminates replay at the torn tail
  (WAL-011) instead of silently misrouting or dropping the entry. Closed before
  the format freeze.
- **Decode OOM** (phase2e): `IdDir::decode` bounded its bucket pre-allocation by
  the input length, so an adversarial `bucket_count` no longer triggers a
  multi-GiB allocation (decode-fuzz could abort with an out-of-memory on
  arbitrary bytes — a corrupt file must fail with `Corrupt`, never OOM).

### Changed
- **PRD OQ-1 resolved** (phase1d): the reference hardware profile is pinned in
  [docs/benchmarks.md](docs/benchmarks.md) — a desktop AMD Ryzen 9 7950X3D class
  and an AWS `c7a.4xlarge` cloud class.
- **ADR-0001**: VecLite has zero dependency on Vectorizer crates. The originally
  planned `vectorizer-core` dependency (unpublished; mandatory network deps conflict
  with NFR-08) is replaced by a vendoring policy — needed code is copied into this
  repo with provenance headers, byte-identical encodings enforced by the conformance
  corpus. Quantization/SIMD land with `phase1b`, compression with `phase2a`.
- **ADR-0003** (phase2c): memory-mapped larger-than-RAM reads (STG-004) and
  HNSW-graph persistence (STG-063) are deferred while `hnsw_rs =0.3.4` is the
  index — it keeps a full f32 copy of every vector in RAM (so mmap gives no
  larger-than-RAM benefit) and has no stable graph serialization (so the graph
  is rebuilt from vectors on every open). Both are tracked in
  `phase2f_mmap-hnsw-persistence`, gated on an index-strategy decision.
- **MSRV raised to 1.87** (phase2e, REL-002): edition 2024 floors at 1.85, but
  the pinned `hnsw_rs =0.3.4` (CORE-030) uses `is_multiple_of` (stable in 1.87),
  so 1.87 is the true minimum. `Cargo.toml` `rust-version`, SPEC-016 REL-002,
  and the MSRV CI job updated accordingly.
- **vacuum mechanism** (phase2d): v1 `vacuum()` shrinks via a compacted
  temp-file + atomic close→rename→reopen swap (SQLite-VACUUM style) rather than
  the in-place append-then-truncate of SPEC-002 STG-071. This is crash-safe and
  Windows-safe without an active memory map; the STG-071 "unmap→truncate→remap"
  in-place variant only becomes relevant once mmap lands, and is tracked in
  `phase2f_mmap-hnsw-persistence` alongside the active-mmap vacuum test (was
  phase2d 2.3).
