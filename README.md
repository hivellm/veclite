# VecLite

[![Rust](https://img.shields.io/badge/rust-1.87%2B-orange.svg)](https://www.rust-lang.org/)
[![Rust Edition](https://img.shields.io/badge/edition-2024-blue.svg)](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)
[![License](https://img.shields.io/badge/license-Apache--2.0-green.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-pre--release-yellow.svg)](#-status--roadmap)

**Embedded, single-file, in-process vector database.** VecLite is the in-process distribution of the [Vectorizer](https://github.com/hivellm/vectorizer) engine: HNSW search, quantization, hybrid dense+sparse retrieval, and payload filtering — as a library you link, not a server you run.

> Reference point: VecLite follows the embedded-database philosophy popularized by SQLite — one linked library, one file, zero configuration.

```python
import veclite

db = veclite.open("app.veclite")
docs = db.create_collection("docs", auto_embed="bm25", dimension=512)
docs.upsert_text("readme", open("README.md").read(), {"lang": "en"})
hits = docs.search_text("how do I configure logging", limit=5)
```

No server. No ports. No configuration. One file.

## ✨ Key Features

### Storage & Durability
- **Single-file `.veclite` format v1** — 4 KiB header, immutable CRC'd segments, MessagePack TOC, root-pointer-swap commit ([SPEC-002](docs/specs/SPEC-002-storage-format.md)). **Frozen-normative**: the byte format is fixed, guarded by committed golden files.
- **Write-ahead log** with three durability modes, checkpointing, and crash recovery ([SPEC-003](docs/specs/SPEC-003-wal-durability.md)) — kill-9 never corrupts the file.
- **Crash suite** — randomized workloads against an oracle model, torn-write and bit-flip sweeps, and a real subprocess kill-9 harness (`cargo xtask crash`, 10,000 iterations nightly on Linux/macOS/Windows).
- **Memory-mapped reads** ([ADR-0004](.rulebook/decisions/004-single-file-mmap-vectors-with-exact-brute-force-larger-than-ram-tier.md)) — big collections keep vectors in a read-only map of the same file. Under `OpenOptions::memory_budget` the HNSW graph rebuilds from the map on open; past it, searches run as exact SIMD scans — datasets larger than RAM open and serve with exact recall.
- **Snapshot & vacuum** — `db.snapshot(path)` writes a standalone compacted copy without blocking writers; `db.vacuum()` reclaims dead space in place and escalates automatically past the tombstone threshold.
- **Advisory locking** — exclusive read-write / shared read-only; a second opener fails fast with `Locked` instead of corrupting. `OpenOptions::read_only(true)` serves reads while rejecting writes.

### Search
- **HNSW indexing** — Cosine / Euclidean, soft-delete tombstones, `reindex()`; ranked `Hit`s ordered per metric.
- **Hybrid search** — `hybrid_query()` fuses a dense and a sparse lane with deterministic Reciprocal Rank Fusion; `alpha`/`rrf_k` tunable, single lane degenerates to plain search. `hybrid_query().text(q)` fills both lanes from one string on auto-embed collections. Fused rankings pinned by a committed conformance corpus.
- **Payload filters** — Qdrant-style `Filter { must, should, must_not }` with server-parity semantics, built in Rust or parsed from portable JSON, applied via `query(v).filter(f)` ([SPEC-006](docs/specs/SPEC-006-payload-filters.md)).
- **Payload indexes** — declared `Keyword`/`Integer`/`Float` indexes accelerate filtered queries (results identical to a scan), addable at runtime with `create_payload_index` (journaled + sealed, surviving crash and reopen). A selectivity planner picks between exact pre-filter and HNSW over-fetch post-filter with an exact fallback.
- **Quantization** — SQ-8 / scalar / binary, vendored byte-identical from the server; scalar SIMD distance kernels.

### Embeddings & Text
- **Auto-embed collections** — `upsert_text`/`search_text` turn text into vectors offline with pure-Rust sparse providers: `bm25` (default), `tfidf`, `bow`, `char_ngram`, plus `svd` behind the `svd` feature ([SPEC-005](docs/specs/SPEC-005-embeddings.md)).
- **Persistent vocabulary** — updates incrementally and persists in the file; reopen searches identically with no rebuild.
- **ONNX dense embeddings** — opt-in `onnx` feature runs local sentence-transformer models via fastembed for dense auto-embed collections; the default build stays pure-Rust.
- **Custom providers** — plug in per database via `register_embedder`.
- **Server parity** — provider outputs pinned to the Vectorizer server within 1e-5 by a generated parity corpus.
- **Chunking** — a deterministic, UTF-8-safe `chunk::Chunker` splits long text.

### API
- **Collection registry** — create / get / delete / rename, plus **aliases** for blue-green swaps.
- **Vector CRUD** — upsert / get / delete, single and batch, with dimension and NaN/Inf guards and cosine ingest normalization.
- **Operational surface** — cursor-based `scroll`, parallel `search_batch`, `stats()`.
- **Portable image codec** — a `.veclite` v1 file assembled in / parsed from bytes with no filesystem: the WASM persistence path, byte-compatible with native files by construction.

### Interop & Tooling
- **`.vecdb` interop** (`vecdb-interop` feature) — the graduation path ([SPEC-013](docs/specs/SPEC-013-vecdb-interop.md)): export to the Vectorizer server's Compact layout (accepted by the server's own reader, vocabulary and scoring intact), and import both server layouts (Compact + Legacy) with an explicit degradation matrix — warnings, never silent; encrypted payloads refused; server-only providers fall back to BYO-vector with the origin recorded. Gate: top-10 overlap ≥ 0.99 and BM25 parity within 1e-5, automated by `cargo xtask graduation` against the pinned server's code (shared conformance corpus in both repos).
- **`veclite` CLI** (`veclite-cli` crate) — `inspect` / `export` / `import` / `vacuum` / `snapshot` / `verify` with stable exit codes (0 success · 1 integrity · 2 usage · 3 environment), `--json` on inspect, no network ([SPEC-014](docs/specs/SPEC-014-cli.md)). `verify` runs a full read-only integrity pass naming each damaged segment by offset and type.

### Bindings
- **C ABI** (`veclite-ffi`) — panic-safe, cbindgen-generated header with a committed golden-file drift test ([SPEC-008](docs/specs/SPEC-008-ffi-c-abi.md)).
- **Python** (`veclite-py`) — PyO3 with NumPy zero-copy and GIL release, `pip install`-able abi3 wheels ([SPEC-009](docs/specs/SPEC-009-binding-python.md)).
- **Node.js** (`veclite-node`) — native addon with prebuilds and a conformance suite ([SPEC-010](docs/specs/SPEC-010-binding-node.md)).
- **Go & C#** — bindings over the C ABI, each with a conformance runner ([SPEC-011](docs/specs/SPEC-011-bindings-go-csharp.md)).
- **WASM** (`@veclite/wasm`) — client-side vector search: the pure-Rust in-memory engine plus the portable image codec; an image written in the browser opens with native VecLite ([SPEC-012](docs/specs/SPEC-012-binding-wasm.md)).

## 🚧 Status & Roadmap

Pre-1.0, built phase by phase against a normative spec set. Format v1 is **frozen at gate G2** — files written today stay readable.

| Phase | Scope | Status |
|---|---|---|
| 0–1 | Foundation + in-memory engine (registry, CRUD, HNSW, quantization) | ✅ Complete |
| 2 | Persistence: `.veclite` v1, WAL, crash recovery, locks, snapshot/vacuum, mmap | ✅ Complete (format frozen) |
| 3 | Payload filters + indexes, auto-embed, hybrid RRF, API surface | ✅ Complete |
| 4 | Bindings: C ABI, Python, Node.js (+ prebuilds & conformance) | ✅ Complete |
| 5a | Go & C# bindings over the C ABI | ✅ Complete |
| 5b | WASM package + portable `.veclite` image codec | ✅ Complete |
| 5c | Opt-in ONNX dense embeddings (fastembed) | ✅ Complete |
| 5d | `.vecdb` interop + `veclite` CLI | ✅ Complete |
| 6 | Fuzz/soak hardening · docs & benchmark report · release 1.0 | Planned |

Scoped forward per the [DAG](docs/DAG.md): SIMD ISA backends (scalar oracle ships now) and `DotProduct` HNSW (served by exact brute force — blocked by the pinned `anndists` dot-bound).

## 🚀 Quick Start

### Rust

```rust
use veclite::{CollectionOptions, VecLite};

let db = VecLite::open("app.veclite")?;
let docs = db.create_collection("docs", CollectionOptions::auto_embed("bm25", 512))?;
docs.upsert_text("readme", std::fs::read_to_string("README.md")?)?;
let hits = docs.search_text("how do I configure logging", 5)?;
```

Bring your own vectors instead:

```rust
use veclite::{Condition, Filter, Metric, Point};

let vecs = db.create_collection("vecs", CollectionOptions::new(384, Metric::Cosine))?;
vecs.upsert(Point::new("a", embedding).payload(json!({ "lang": "en" })))?;
let hits = vecs
    .query(&query_vec)
    .filter(Filter::new().must(Condition::eq("lang", "en")))
    .limit(5)
    .run()?;
```

### Development

```bash
cargo check --workspace                                        # diagnostics first
cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features                          # unit + doc tests
cargo build --workspace --target wasm32-unknown-unknown        # wasm32 gate (CORE-004)
```

Task backlog lives in `.rulebook/tasks/` (one task per context cycle); the implementation contract is in [`docs/specs/`](docs/specs/README.md).

## 🔄 Relationship to Vectorizer

| | Vectorizer | VecLite |
|---|---|---|
| Model | client-server | embedded library |
| Storage | managed `data/` dir | single `.veclite` file |
| Access | REST / RPC / gRPC / MCP | function calls |
| Scale | replication, Raft, sharding | one process |
| Use | shared production infra | CLIs, apps, agents, edge, tests |

Same math, same defaults, same concepts — start embedded, graduate to the server when you need the network. Quantization and SIMD kernels are vendored byte-identical from the server; embedding providers are pinned to server outputs by a parity corpus.

## 🏗️ Workspace Layout

```
crates/
├── veclite/          # Engine: HNSW, .veclite format v1, WAL, filters, embeddings, hybrid
├── veclite-cli/      # `veclite` binary: inspect/export/import/vacuum/snapshot/verify
├── veclite-ffi/      # Panic-safe C ABI (cbindgen header + golden-file drift test)
├── veclite-py/       # Python binding (PyO3, abi3 wheels, NumPy zero-copy) — maturin-built
├── veclite-node/     # Node.js binding (native addon + prebuilds)
└── veclite-wasm/     # WASM binding (@veclite/wasm) — wasm-bindgen, portable image codec
xtask/                # Dev tool: crash harness (`cargo xtask crash`) and friends
```

`veclite-py`, `veclite-node`, and `veclite-wasm` are excluded from `--workspace` jobs (they link non-Rust runtimes); default commands target the library.

## 📚 Documentation

- **Implementation contract**: [`docs/specs/`](docs/specs/README.md) — [PRD](docs/PRD.md) (requirements & release criteria), [DAG](docs/DAG.md) (task dependency graph), and SPEC-001…016 (normative component specs).
- **Design rationale**: [`docs/vectorizer-lite/`](docs/vectorizer-lite/README.md), the original planning set:
  - [Vision and scope](docs/vectorizer-lite/01-vision-and-scope.md) — what's in, what's out, feature matrix vs Vectorizer
  - [Architecture](docs/vectorizer-lite/02-architecture.md) — crate layout, reuse of `vectorizer-core`, design decisions
  - [Core API](docs/vectorizer-lite/03-core-api.md) — Rust API (source of truth for all bindings)
  - [Storage format](docs/vectorizer-lite/04-storage-format.md) — single-file `.veclite`, WAL, crash safety
  - [Embeddings](docs/vectorizer-lite/05-embeddings.md) — BYO vectors, built-in BM25/TF-IDF, optional ONNX
  - [SDK bindings](docs/vectorizer-lite/06-sdk-bindings.md) — Python, Node.js, Go, C#, WASM (native, not network)
  - [Vectorizer compatibility](docs/vectorizer-lite/07-vectorizer-compatibility.md) — graduation path to the server
  - [Roadmap](docs/vectorizer-lite/08-roadmap.md) — phases to 1.0

## 📄 License

Apache License 2.0 (matching Vectorizer) — see [LICENSE](LICENSE).
