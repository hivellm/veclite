# 01 — Vision and Scope

## Vision

VecLite is the embedded distribution of the Vectorizer engine. It must feel like SQLite: a single dependency, a single file, zero configuration, and the full power of the engine available through an idiomatic API in each host language.

A developer should be able to go from nothing to semantic search in under five lines:

```python
import veclite

db = veclite.open("app.veclite")
docs = db.create_collection("docs", dimension=512)          # BM25 auto-embedding by default
docs.upsert_texts([("readme", open("README.md").read(), {"lang": "en"})])
hits = docs.search_text("how do I configure logging", limit=5)
```

## Goals

1. **Embed the real engine.** VecLite reuses Vectorizer's core algorithms (HNSW via `hnsw_rs 0.3`, SIMD distance kernels, scalar/product/binary quantization, LZ4/zstd compression) — not a simplified reimplementation. Search quality and performance characteristics match the server for single-process workloads.
2. **Single-file storage.** One `.veclite` file per database (plus a transient `-wal` sidecar during writes). Copyable, versionable, streamable.
3. **No server, no async runtime.** The core API is synchronous and thread-safe. Host applications never start tokio, open ports, or manage a daemon.
4. **Native SDKs, not network SDKs.** Python/Node/Go/C#/WASM bindings link the compiled Rust core directly (PyO3, napi-rs, cgo/C-ABI, P/Invoke, wasm-bindgen). Zero network hops.
5. **Small default footprint.** Default build: pure Rust, no ONNX runtime, no model downloads. Target < 10 MB compiled library. Heavy features (dense neural embeddings) are opt-in.
6. **Graduation path.** Data and concepts map 1:1 to Vectorizer server; an app that outgrows VecLite migrates by exporting/importing data and swapping the SDK — collection configs, distance metrics, and HNSW parameters carry over unchanged.

## Non-goals

- **Not a server.** No ports, no REST/gRPC/MCP/GraphQL, no dashboard. If you need remote access, use Vectorizer.
- **Not multi-process write-shared.** Like SQLite in WAL mode, VecLite supports one writer process with multi-threaded access; concurrent writer *processes* are out of scope for 1.0 (advisory file lock enforces this).
- **Not multi-tenant.** No auth, no API keys, no quotas, no RBAC. The process boundary is the security boundary.
- **Not distributed.** No replication, no Raft, no sharding, no HA. One file, one process.
- **Not an ingestion platform.** No file watcher daemon, no workspace indexing pipeline, no document-conversion service (the server's `transmutation`, `discovery/`, `file_watcher/` stay server-side). VecLite offers a minimal chunker utility and that's it.

## Feature matrix

Legend: ✅ included in VecLite core · 🔌 optional cargo feature / extra · ❌ excluded (server-only) · Source column = where the code lives in the Vectorizer workspace today.

### Core engine

| Feature | VecLite | Source in Vectorizer | Notes |
|---|---|---|---|
| Collections (create/delete/rename/list) | ✅ | `crates/vectorizer/src/db/vector_store/collections.rs` | Drop owner/tenant variants |
| Vector CRUD (insert/upsert/get/update/delete) | ✅ | `db/vector_store/vectors.rs`, `db/collection/data.rs` | Incl. `insert_batch` |
| k-NN search (HNSW) | ✅ | `db/optimized_hnsw.rs`, `hnsw_rs 0.3` | Same `m` / `ef_construction` / `ef_search` params |
| Distance metrics: Cosine, Euclidean, DotProduct | ✅ | `models/mod.rs` (`DistanceMetric`) | |
| SIMD distance kernels | ✅ | `vectorizer-core/src/simd/` | AVX2/NEON/WASM, runtime detection |
| Payload (metadata) storage per vector | ✅ | `models/` (`Payload`) | JSON payloads |
| Payload filters (must/should/must_not; match/range) | ✅ | `db/payload_index.rs`, `models/qdrant/filter.rs`, `filter_processor.rs` | Keyword + numeric range indexes; geo filters deferred to post-1.0 |
| Hybrid search (dense + sparse, RRF) | ✅ | `db/hybrid_search.rs`, `models/sparse_vector.rs` | |
| Batch operations (insert/update/delete/search) | ✅ | `crates/vectorizer/src/batch/` | Simplified: no atomicity flags in v1, whole-batch WAL entry |
| Collection aliases | ✅ | `db/vector_store/aliases.rs` | Cheap, useful for blue/green reindex |
| Scalar quantization (SQ-8/4/2/1) | ✅ | `vectorizer-core/src/quantization/scalar.rs` | 4× memory savings at 8-bit; **on by default** (matches server default `SQ { bits: 8 }`) |
| Binary quantization | ✅ | `vectorizer-core/src/quantization/binary.rs` | |
| Product quantization (PQ) | 🔌 `pq` | `vectorizer-core/src/quantization/product.rs` | Training cost; niche for embedded |
| Query cache (LRU) | 🔌 `cache` | `crates/vectorizer/src/cache/query_cache.rs` | Off by default; embedded callers can cache themselves |
| Snapshots (point-in-time copy) | ✅ | `db/vector_store/persistence.rs`, `storage/snapshot.rs` | `db.snapshot("path")` — single-file copy with compaction |
| Reindex (rebuild HNSW) | ✅ | `db/vector_store/persistence.rs` (`reindex_collection`) | |
| Search explain / trace | 🔌 `explain` | `db/vector_store/search.rs` (`search_explained`) | Debug builds |
| Graph relationships (edges, neighbors, paths) | ❌ v1 → 🔌 post-1.0 | `db/` graph modules | Revisit after 1.0 |

### Storage

| Feature | VecLite | Source | Notes |
|---|---|---|---|
| Single-file database | ✅ | Adapted from `storage/` (`.vecdb` writer/reader/index) | `.vecidx` sidecar folded into the file — see [04-storage-format.md](04-storage-format.md) |
| WAL + crash recovery | ✅ | `persistence/wal.rs`, `db/wal_integration.rs` | `-wal` sidecar, auto-checkpoint; **on by default** (server default is off) |
| Memory-mapped reads (larger-than-RAM) | ✅ | `storage/mmap.rs` (`memmap2`) | `StorageType::Mmap` equivalent |
| LZ4/zstd block compression | ✅ | `vectorizer-core` compression | LZ4 default, threshold 1 KiB (server parity) |
| Checksums (crc32) | ✅ | `crc32fast` usage in `storage/` | Per-segment |
| Compaction | ✅ | `storage/compact.rs` | Explicit `db.vacuum()` + auto-threshold |
| Import/export `.vecdb` (server format) | 🔌 `vecdb-interop` | `storage/migration.rs` | Graduation path — see [07](07-vectorizer-compatibility.md) |

### Embeddings

| Feature | VecLite | Source | Notes |
|---|---|---|---|
| Bring-your-own-vectors | ✅ | — | First-class: most users embed with their own model/API |
| BM25 (sparse, vocabulary-based) | ✅ | `embedding/providers/bm25.rs` | Default auto-embedding provider (server parity) |
| TF-IDF / BoW / char-n-gram | ✅ | `embedding/providers/{tfidf,bag_of_words,char_ngram}.rs` | Pure Rust, no deps |
| SVD-reduced TF-IDF | 🔌 `svd` | `embedding/providers/svd.rs` | Pulls `ndarray` |
| Text chunker utility | ✅ | `file_loader/chunker.rs` | UTF-8-safe, sentence-boundary |
| ONNX dense models (fastembed / ort) | 🔌 `onnx` | `embedding/providers/fastembed.rs`, `embedding/onnx_models.rs` | Heavy: ONNX Runtime + model downloads; **never** in default build |
| Candle models | ❌ | `embedding/real_models.rs` | Server-only; ONNX path covers the embedded need |
| OpenAI / remote embedding APIs | ❌ | `embedding/openai.rs` | Out of scope — callers can do HTTP themselves and pass vectors in |
| Vocabulary persistence (BM25/TF-IDF state) | ✅ | `EmbeddingProvider::save/load_vocabulary_json` | Stored inside the `.veclite` file per collection |

### Excluded entirely (server concerns)

| Feature | Where it lives today | Reason for exclusion |
|---|---|---|
| REST / gRPC / MCP / GraphQL / WS / RPC-TCP | `crates/vectorizer-server/` | Transport = server's job; VecLite is a function call |
| Dashboard / Desktop GUI | `dashboard/`, `gui/` | UI over network APIs |
| Auth (JWT, API keys, RBAC, audit) | `crates/vectorizer/src/auth/` | Process boundary is the security boundary |
| Payload encryption (ECC-P256 + AES-GCM) | `src/security/` | Encrypt-at-rest belongs to the host app / OS (post-1.0 candidate if demanded) |
| Replication (master-replica) | `src/replication/` | Distributed concern |
| Raft cluster / sharding / HA | `src/cluster/`, `db/sharded_collection.rs`, `db/raft.rs` | Distributed concern |
| Multi-tenancy / quotas | `db/multi_tenancy.rs` | Single-app database |
| Monitoring (Prometheus/OTel) | `src/monitoring/` | Host app instruments itself; VecLite exposes plain stats structs |
| File watcher / discovery pipeline | `src/file_watcher/`, `src/discovery/` | Ingestion platform, not engine |
| Summarization (incl. OpenAI abstractive) | `src/summarization/` | LLM orchestration, not storage/search |
| Document conversion (PDF/DOCX/…) | `transmutation` feature | Heavy native deps; users pre-convert |
| Qdrant API compatibility layer | `models/qdrant/*` (kept only for filter model), `vectorizer-server` | Wire-protocol compat is meaningless without a wire; the **filter data model** is retained because the engine uses it internally |

## Success criteria for 1.0

1. `cargo add veclite` → open/insert/search works with zero config; default build compiles in < 60 s clean on a laptop, produces < 10 MB rlib-linked binary overhead.
2. Python (`pip install veclite`) and Node (`npm install veclite`) wheels/prebuilds for the big-3 OS × x64/arm64, installing with **no** Rust toolchain.
3. 1 M × 512-dim vectors: p50 search < 3 ms, index build within 2× of server single-node time, file open (mmap, warm) < 100 ms.
4. Crash-kill test suite: `kill -9` at any point never corrupts the file (WAL replay recovers or discards cleanly).
5. Round-trip `veclite export --vecdb` → Vectorizer server imports and serves identical search results (top-10 overlap ≥ 0.99 on the standard benchmark set).
