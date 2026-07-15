# VecLite

**Embedded, single-file, in-process vector database.**

> Reference point: VecLite follows the embedded-database philosophy popularized by SQLite — one linked library, one file, zero configuration.

VecLite is the in-process, single-file distribution of the [Vectorizer](https://github.com/hivellm/vectorizer) engine: HNSW search, quantization, hybrid dense+sparse retrieval, and payload filtering — as a library you link, not a server you run.

```python
import veclite

db = veclite.open("app.veclite")
docs = db.create_collection("docs", auto_embed="bm25", dimension=512)
docs.upsert_text("readme", open("README.md").read(), {"lang": "en"})
hits = docs.search_text("how do I configure logging", limit=5)
```

No server. No ports. No configuration. One file.

## Status

🚧 **Phase 1 complete; Phase 2 (persistence) complete — format v1 frozen at gate G2.** `VecLite::open(path)` opens a durable single-file database: the `.veclite` format v1 (4 KiB header, immutable CRC'd segments, MessagePack TOC, root-pointer-swap commit — [SPEC-002](docs/specs/SPEC-002-storage-format.md)) plus a write-ahead log ([SPEC-003](docs/specs/SPEC-003-wal-durability.md)) with three durability modes, checkpointing, and crash recovery — kill-9 never corrupts the file. The crash suite proves it: randomized workloads with an oracle model, torn-write and bit-flip sweeps, and a real subprocess kill-9 harness (`cargo xtask crash`, 10 000 iterations nightly on Linux/macOS/Windows), with committed v1 golden files guarding the freeze. **SPEC-002/SPEC-003 are frozen-normative** — the byte format is fixed (phase 2e). Search now takes payload filters (phase 3a): a Qdrant-style `Filter { must, should, must_not }` with server-parity semantics, built in Rust or parsed from portable JSON, applied via `query(v).filter(f)`; declared `Keyword`/`Integer`/`Float` payload indexes pre-filter selective queries as accelerators (results identical to a scan). Auto-embed collections (phase 3b) turn text into vectors offline with pure-Rust sparse providers (`bm25` default, plus `tfidf`/`bow`/`char_ngram`): `upsert_text`/`search_text` embed with the collection's provider and stay identical across a reopen, and a UTF-8-safe `chunk::Chunker` splits long text. `hybrid_query()` fuses a dense and a sparse (BYO) lane with deterministic reciprocal rank fusion (phase 3c), matching the server's `alpha`/`rrf_k` semantics; a single lane degenerates to plain search. The API surface is rounding out (phase 3d): collection aliases for blue-green swaps, cursor-based `scroll`, parallel `search_batch`, and `stats()`. Bindings are underway: a panic-safe C ABI (`veclite-ffi`, phase 4a) and a PyO3 Python binding with NumPy zero-copy and GIL release (`veclite-py`, phase 4b, `pip install`-able abi3 wheels). Opening takes an advisory lock (exclusive read-write, shared read-only) so a second opener fails fast with `Locked` instead of corrupting; `OpenOptions::read_only(true)` serves reads and searches while rejecting writes with `ReadOnly` (phase 2c). `db.snapshot(path)` writes a standalone compacted copy without blocking writers, and `db.vacuum()` reclaims dead space in place — shrinking the file after bulk deletes and escalating automatically past the tombstone threshold (phase 2d). Memory-mapped larger-than-RAM reads and HNSW-graph persistence are tracked in phase 2f, gated on an index-strategy decision ([ADR-0003](.rulebook/decisions/003-mmap-as-primary-store-and-hnsw-graph-persistence-deferred-while-hnsw-rs-is-the-index.md)).

**Phase 1 — in-memory engine + HNSW.** On top of the phase 0 foundation (`VecLiteError`, options with server-parity defaults, CI gates), an ephemeral `VecLite::memory()` database runs the collection registry (create/get/delete/rename) and vector CRUD (upsert/get/delete, single and batch) with dimension and NaN/Inf guards and cosine ingest normalization. Vectors are indexed in an HNSW graph (Cosine/Euclidean) with soft-delete tombstones and `reindex()`, and `search(vector, limit)` / `query(vector)…run()` return ranked `Hit`s ordered per metric; quantization (SQ-8/scalar/binary) and scalar SIMD distance kernels are vendored byte-identical from the server. A few items are scoped forward per the [DAG](docs/DAG.md): SIMD ISA backends (scalar oracle ships now), `DotProduct` HNSW (served by exact brute force for now — blocked by the pinned `anndists` dot-bound), payload filters (`filter` slot declared, evaluation in phase 3a), and in-memory quantized storage (the live graph is f32; SQ-8 is the on-disk/interop encoding, realized with phase 2 segments).

### Development

```bash
cargo check --workspace                                        # diagnostics first
cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features                          # unit + doc tests
cargo build --workspace --target wasm32-unknown-unknown        # wasm32 gate (CORE-004)
```

Task backlog lives in `.rulebook/tasks/` (one task per context cycle, `phase0a…phase6c`); the implementation contract is below.

- **Implementation contract**: [`docs/specs/`](docs/specs/README.md) — [PRD](docs/PRD.md) (requirements & release criteria), [DAG](docs/DAG.md) (task dependency graph), and SPEC-001…016 (normative component specs).
- **Design rationale**: [`docs/vectorizer-lite/`](docs/vectorizer-lite/README.md), the original planning set:

1. [Vision and scope](docs/vectorizer-lite/01-vision-and-scope.md) — what's in, what's out, feature matrix vs Vectorizer
2. [Architecture](docs/vectorizer-lite/02-architecture.md) — crate layout, reuse of `vectorizer-core`, design decisions
3. [Core API](docs/vectorizer-lite/03-core-api.md) — Rust API (source of truth for all bindings)
4. [Storage format](docs/vectorizer-lite/04-storage-format.md) — single-file `.veclite`, WAL, crash safety
5. [Embeddings](docs/vectorizer-lite/05-embeddings.md) — BYO vectors, built-in BM25/TF-IDF, optional ONNX
6. [SDK bindings](docs/vectorizer-lite/06-sdk-bindings.md) — Python, Node.js, Go, C#, WASM (native, not network)
7. [Vectorizer compatibility](docs/vectorizer-lite/07-vectorizer-compatibility.md) — graduation path to the server
8. [Roadmap](docs/vectorizer-lite/08-roadmap.md) — phases to 1.0

## Relationship to Vectorizer

| | Vectorizer | VecLite |
|---|---|---|
| Model | client-server | embedded library |
| Storage | managed `data/` dir | single `.veclite` file |
| Access | REST / RPC / gRPC / MCP | function calls |
| Scale | replication, Raft, sharding | one process |
| Use | shared production infra | CLIs, apps, agents, edge, tests |

Same math, same defaults, same concepts — start embedded, graduate to the server when you need the network.

## License

Apache-2.0 (matching Vectorizer).
