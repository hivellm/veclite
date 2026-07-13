# 02 — Architecture

## Overview

```
Host application (Rust / Python / Node / Go / C# / WASM)
        │  function calls (no network, no IPC)
        ▼
┌───────────────────────────────────────────────┐
│ veclite (core crate, sync, thread-safe)       │
│                                               │
│  Database ──► Collection ──► HNSW index       │
│     │             │             │             │
│     │             ├─ payload index (filters)  │
│     │             ├─ sparse index (BM25)      │
│     │             └─ embedding provider       │
│     │                                         │
│  storage engine: single .veclite file         │
│  (segments + WAL sidecar + mmap reads)        │
└───────────────────────────────────────────────┘
        │ depends on
        ▼
  vectorizer-core (shared with the server:
  quantization, SIMD kernels, compression, codec)
```

## Repository layout (new repo: `hivellm/veclite`)

```
veclite/
├── Cargo.toml                  # workspace
├── crates/
│   ├── veclite/                # core library (the product)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── database.rs     # Database handle, open/close/snapshot/vacuum
│   │       ├── collection.rs   # Collection handle, CRUD + search
│   │       ├── options.rs      # CollectionOptions, OpenOptions, defaults
│   │       ├── error.rs        # VecLiteError (thiserror)
│   │       ├── index/          # HNSW wrapper (from db/optimized_hnsw.rs)
│   │       ├── filter/         # payload index + filter evaluation
│   │       ├── hybrid/         # sparse vectors + RRF fusion
│   │       ├── embedding/      # BM25/TF-IDF/BoW/char-ngram + trait
│   │       ├── chunk.rs        # text chunker utility
│   │       └── storage/        # .veclite format: pager, segments, WAL, mmap
│   ├── veclite-ffi/            # C ABI (cdylib + header) — feeds Go/C#/others
│   ├── veclite-py/             # PyO3 + maturin
│   ├── veclite-node/           # napi-rs
│   └── veclite-wasm/           # wasm-bindgen (browser/edge profile)
├── bindings/
│   ├── go/                     # cgo wrapper over veclite-ffi
│   └── csharp/                 # P/Invoke wrapper over veclite-ffi
├── docs/
│   └── vectorizer-lite/        # this documentation
└── tests/
    ├── crash/                  # kill -9 durability harness
    ├── compat/                 # .vecdb import/export round-trips
    └── bench/                  # criterion benchmarks vs targets
```

## Code reuse strategy

### Decision: depend on `vectorizer-core`, fork the rest

The Vectorizer workspace already split into crates (phase4): `vectorizer-core` holds exactly the layer VecLite needs to share — error/codec primitives, **quantization** (scalar/product/binary), **SIMD** distance kernels, **compression** (LZ4/zstd), path helpers. It has no tokio, no axum, no server dependency.

| Layer | Strategy | Rationale |
|---|---|---|
| `vectorizer-core` (quantization, SIMD, compression, codec) | **Depend on the published crate** | Bit-identical quantization/distance math guarantees search-result parity and `.vecdb` interop; bug fixes flow both ways through one crate |
| Engine logic (`db/`: collection, HNSW wrapper, payload index, hybrid search) | **Extract-and-adapt** (copy into `veclite`, strip async/server/tenant/shard branches) | The server's `VectorStore` is entangled with `CollectionType` dispatch (GPU/Sharded/Distributed variants), tokio, and multi-tenancy. VecLite needs the CPU path only, synchronous. A shared "engine" crate is the right long-term move but premature — revisit after VecLite 1.0 (see [08-roadmap.md](08-roadmap.md)) |
| Embedding providers (BM25/TF-IDF/BoW/char-ngram) | **Extract-and-adapt** | Small, pure-Rust files; trait shrinks to sync-only |
| Storage (`storage/`, `persistence/wal.rs`) | **Redesign** (new single-file format informed by `.vecdb`) | `.vecdb` uses a sidecar `.vecidx` JSON index + snapshots directory — three artifacts. VecLite's contract is ONE file; see [04-storage-format.md](04-storage-format.md) |
| Qdrant filter model (`models/qdrant/filter.rs`, `filter_processor.rs`) | **Extract-and-adapt** | Proven filter semantics (must/should/must_not, match/range); drop geo + nested for v1 |
| Everything in `vectorizer-server`, `auth/`, `cluster/`, `replication/`, `monitoring/`, `discovery/` | **Not imported** | Out of scope ([01-vision-and-scope.md](01-vision-and-scope.md)) |

Alternative considered and rejected: *make VecLite a cargo feature of the `vectorizer` crate* (`default-features = false`). Rejected because the umbrella crate compiles auth/cluster/replication as unconditional modules (not feature-gated), carries 100+ dependencies, and its API is `async` + `Arc<DashMap>`-shaped. Decoupling in-place would be a bigger refactor than extraction, on a crate that ships server releases on its own cadence.

## Core design decisions

### D1 — Synchronous core, no tokio

SQLite's lesson: an embedded library must not impose a runtime. All core operations are blocking; internal parallelism (batch inserts, HNSW build) uses `rayon` scoped threads. Async facades live in the bindings where the host platform expects them (Node's napi-rs async tasks, Python's optional `asyncio` wrapper running calls on a thread pool).

### D2 — Thread-safe handles, single-writer semantics

- `Database` is `Send + Sync`; cheap to clone (internal `Arc`).
- Reads are lock-free on the hot path (`DashMap` for collection registry, `parking_lot::RwLock` per collection — same primitives as the server).
- Writes within a process: serialized per collection by the WAL appender.
- Across processes: an advisory file lock (`fs2`/`fd-lock`) makes a second `open()` for write fail fast with `Error::Locked` (read-only open of a quiesced file is allowed). Matches SQLite's practical single-writer posture.

### D3 — In-memory mode

`VecLite::memory()` — no file, no WAL, same API. Critical for tests and ephemeral agent workloads; mirrors `:memory:` in SQLite. Server parity note: this is `StorageType::Memory` semantics with persistence disabled.

### D4 — Collections own their embedding state

A collection created with `auto_embed("bm25")` persists its vocabulary inside the file (the server stores vocabulary JSON via `save_vocabulary_json`). Reopening the file restores identical embedding behavior — a `.veclite` file is fully self-contained: config + vectors + indexes + payloads + embedding state.

### D5 — Quantization on by default (SQ-8)

Server default is `SQ { bits: 8 }` (4× memory reduction, ~0.99 recall). VecLite keeps this default for parity and for the embedded reality: RAM is the scarcest resource in-process. `Quantization::None` is one option away.

### D6 — Errors: one `thiserror` enum, no panics

`VecLiteError` with stable, matchable variants (`NotFound`, `DimensionMismatch`, `Locked`, `Corrupt`, `UnsupportedFormatVersion`, `Io`, …). The no-`unwrap`/no-`expect` policy from the Vectorizer workspace (`unwrap_used = "deny"`) carries over. FFI maps variants to error codes ([06-sdk-bindings.md](06-sdk-bindings.md)).

### D7 — Feature flags (cargo)

```toml
[features]
default = ["simd"]          # pure Rust, small, fast to compile
simd    = []                # AVX2/NEON kernels via vectorizer-core
onnx    = ["dep:fastembed"] # dense neural embeddings (heavy, opt-in)
pq      = []                # product quantization
svd     = ["dep:ndarray"]   # SVD-reduced TF-IDF
cache   = []                # LRU query cache
explain = []                # search tracing
vecdb-interop = []          # .vecdb import/export (graduation path)
```

Rule inherited from the SQLite philosophy: **the default build must never download anything, link C++ (ONNX Runtime), or require protoc.**

## Concurrency model (detail)

| Operation | Locking | Notes |
|---|---|---|
| `search` / `get` | read lock (collection) | concurrent with other reads; mmap page-fault reads |
| `insert` / `upsert` / `delete` | short write lock + WAL append | HNSW insert under `RwLock` write guard, batched amortization |
| `create/delete_collection` | registry-level write | rare |
| `vacuum` / `snapshot` | takes a consistent read view | copy-on-write via segment immutability, writers not blocked (see [04](04-storage-format.md)) |
| background work | **none by default** | WAL checkpoint runs opportunistically on write/close; optional `Database::checkpoint()` for explicit control |

No background threads unless the user opts in (`OpenOptions::background_checkpoint(true)`) — an embedded library that spawns threads surprises host runtimes (WASM has none, Python forks break them).

## Anti-goals for the architecture

- No plugin system, no dynamic loading of embedding providers — providers are compile-time features.
- No internal HTTP client. Nothing in core performs network I/O, ever (ONNX feature downloads models via `fastembed` only when explicitly constructing that provider — and even that can be redirected to a local model path for air-gapped use).
- No global state / singletons; multiple `Database` instances in one process are independent.
