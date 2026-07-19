# VecLite

> **Embedded, Single-File, In-Process Vector Database**

[![Rust](https://img.shields.io/badge/rust-1.87%2B-orange.svg)](https://www.rust-lang.org/)
[![Rust Edition](https://img.shields.io/badge/edition-2024-blue.svg)](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)
[![License](https://img.shields.io/badge/license-Apache--2.0-green.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-447%20passing-success.svg)](docs/COVERAGE.md)
[![Specs](https://img.shields.io/badge/specs-16%20documents-blue.svg)](docs/specs/README.md)
[![Format](https://img.shields.io/badge/.veclite-v1%20(frozen)-success.svg)](docs/specs/SPEC-002-storage-format.md)
[![Status](https://img.shields.io/badge/status-pre--release-yellow.svg)](#-status--roadmap)
[![Version](https://img.shields.io/badge/version-0.1.0-blue.svg)](CHANGELOG.md)

VecLite is the in-process distribution of the [Vectorizer](https://github.com/hivellm/vectorizer) engine, written in Rust: HNSW search, quantization, hybrid dense+sparse retrieval, and payload filtering — as a library you link, not a server you run. It follows the embedded-database philosophy popularized by SQLite: one linked library, one file, zero configuration. Ships as a Cargo workspace (6 crates) with a `veclite` CLI and native bindings for Rust, Python, Node.js, Go, C#, and WASM.

> **🚧 Project Status**: **Pre-1.0** (phases 0–5d complete). The `.veclite` v1 format is **frozen at gate G2** — files written today stay readable. Packages are not on public registries yet; build from source. See [Status & Roadmap](#-status--roadmap).

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
- **Adversarial hardening** ([SPEC-015 §6](docs/specs/SPEC-015-testing-conformance.md)) — coverage-guided cargo-fuzz targets over every untrusted-input parser with the corpus replayed on stable in the normal gate (`cargo xtask fuzz`), a sustained-operation soak with oracle invariants and RSS-plateau leak detection incl. an mmap 4×-budget pressure mode (`cargo xtask soak`), ASan/TSan suite runs (`cargo xtask sanitize`), and loom models of the TOC-swap commit protocol.
- **Memory-mapped reads** ([ADR-0004](.rulebook/decisions/004-single-file-mmap-vectors-with-exact-brute-force-larger-than-ram-tier.md)) — big collections keep vectors in a read-only map of the same file. Under `OpenOptions::memory_budget` the HNSW graph rebuilds from the map on open; past it, searches run as exact SIMD scans — datasets larger than RAM open and serve with exact recall.
- **Snapshot & vacuum** — `db.snapshot(path)` writes a standalone compacted copy without blocking writers; `db.vacuum()` reclaims dead space in place and escalates automatically past the tombstone threshold.
- **Advisory locking** — exclusive read-write / shared read-only; a second opener fails fast with `Locked` instead of corrupting. `OpenOptions::read_only(true)` serves reads while rejecting writes.

### Search
- **HNSW indexing** — Cosine / Euclidean, soft-delete tombstones, `reindex()`; ranked `Hit`s ordered per metric.
- **Hybrid search** — `hybrid_query()` fuses a dense and a sparse lane with deterministic Reciprocal Rank Fusion; `alpha`/`rrf_k` tunable, single lane degenerates to plain search. `hybrid_query().text(q)` fills both lanes from one string on auto-embed collections. Fused rankings pinned by a committed conformance corpus ([SPEC-007](docs/specs/SPEC-007-hybrid-search.md)).
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
- **Operational surface** — cursor-based `scroll`, parallel `search_batch`, `stats()` ([SPEC-004](docs/specs/SPEC-004-rust-api.md)).
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

## 🏗️ Architecture

VecLite is a library, so the "transport" layer is a function call. Layers go Foundation → Core → Features → Presentation, matching the HiveLLM family layering.

```
┌──────────────────────────────────────────────────────────────────┐
│  Host process                                                    │
│  Rust · Python · Node.js · Go · C# · WASM · veclite CLI          │
└───────────────────────────────┬──────────────────────────────────┘
                                │  in-process calls (no ports, no IPC)
┌───────────────────────────────▼──────────────────────────────────┐
│  Public API — VecLite / Collection / QueryBuilder                │
│  registry · aliases · CRUD · scroll · search_batch · stats       │
└───────────────────────────────┬──────────────────────────────────┘
┌───────────────────────────────▼──────────────────────────────────┐
│  Features                                                        │
│  HNSW  │  hybrid RRF  │  payload filters  │  payload indexes     │
│  auto-embed (bm25/tfidf/bow/char_ngram/svd · onnx)  │  chunking  │
└───────────────────────────────┬──────────────────────────────────┘
┌───────────────────────────────▼──────────────────────────────────┐
│  Core — quantization (SQ-8/scalar/binary) · SIMD distance        │
│         kernels · vocabulary · .vecdb interop codec              │
└───────────────────────────────┬──────────────────────────────────┘
┌───────────────────────────────▼──────────────────────────────────┐
│  Storage — WAL (3 durability modes) · immutable CRC'd segments   │
│            MessagePack TOC · root-pointer-swap commit · mmap     │
│            advisory locks · snapshot / vacuum                    │
└───────────────────────────────┬──────────────────────────────────┘
                                ▼
                    ┌───────────────────────┐
                    │   app.veclite  (v1)   │   one file, frozen format
                    └───────────────────────┘
```

### Workspace Layout

```
crates/
├── veclite/          # Engine: HNSW, .veclite format v1, WAL, filters, embeddings, hybrid
├── veclite-cli/      # `veclite` binary: inspect/export/import/vacuum/snapshot/verify
├── veclite-ffi/      # Panic-safe C ABI (cbindgen header + golden-file drift test)
├── veclite-py/       # Python binding (PyO3, abi3 wheels, NumPy zero-copy) — maturin-built
├── veclite-node/     # Node.js binding (native addon + prebuilds)
└── veclite-wasm/     # WASM binding (@veclite/wasm) — wasm-bindgen, portable image codec
bindings/
├── go/               # Go binding over the C ABI + conformance runner
└── csharp/           # C# binding over the C ABI + conformance runner
fuzz/                 # cargo-fuzz targets + committed corpus (7 untrusted-input parsers)
xtask/                # Dev tool: crash harness, coverage, graduation gate, fuzz/soak/sanitize
```

`veclite-py`, `veclite-node`, and `veclite-wasm` are excluded from `--workspace` jobs (they link non-Rust runtimes); default commands target the library.

Full design: [`docs/vectorizer-lite/02-architecture.md`](docs/vectorizer-lite/02-architecture.md).

## 🚀 Quick Start

### Installation

VecLite is pre-1.0 and **not yet published to crates.io, PyPI, or npm** — registry publishing lands with 1.0 ([SPEC-016](docs/specs/SPEC-016-packaging-release.md)). Until then, build from source:

```bash
git clone https://github.com/hivellm/veclite.git
cd veclite
cargo build --release
```

| Surface | How to depend on it today |
|---|---|
| 🦀 Rust | `veclite = { git = "https://github.com/hivellm/veclite" }` |
| 🖥️ CLI | `cargo install --path crates/veclite-cli` → `veclite inspect app.veclite` |
| 🐍 Python | `maturin build --release -m crates/veclite-py/Cargo.toml` → `pip install <wheel>` |
| 📘 Node.js | `npm install && npm run build` in `crates/veclite-node/` |
| 🌐 WASM | `wasm-pack build crates/veclite-wasm --target web` |
| 🐹 Go | `go get` the module in `bindings/go/` (links `veclite-ffi`) |
| 💜 C# | Reference the project in `bindings/csharp/` (links `veclite-ffi`) |
| ⚙️ C / C++ | Link `veclite-ffi` with the cbindgen header — see [`docs/c-abi.md`](docs/c-abi.md) |

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

### CLI

```bash
veclite inspect app.veclite --json     # header, collections, segments, dead space
veclite verify  app.veclite            # full read-only integrity pass (exit 1 on damage)
veclite vacuum  app.veclite            # reclaim dead space in place
veclite export  app.veclite out.vecdb  # graduate to the Vectorizer server
```

### Development

```bash
cargo check --workspace                                        # diagnostics first
cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features                          # unit + doc tests
cargo build --workspace --target wasm32-unknown-unknown        # wasm32 gate (CORE-004)
cargo xtask coverage                                           # enforces the coverage floor
cargo xtask crash                                              # crash/kill-9 harness
```

Task backlog lives in `.rulebook/tasks/` (one task per context cycle); the implementation contract is in [`docs/specs/`](docs/specs/README.md).

## 🎯 Use Cases

- **CLI tools & desktop apps** — ship semantic search inside a binary; the index is one file next to the user's data, with no daemon to install or supervise.
- **AI agents & local RAG** — give an agent a persistent memory that survives restarts and travels as a single artifact.
- **Edge & offline** — pure-Rust default build, no network, no runtime deps; the `onnx` feature adds local dense models when you want them.
- **Browser / WASM** — client-side vector search over an index built server-side, byte-compatible with native VecLite via the portable image codec.
- **Tests & CI** — a real vector database in a temp file, deterministic and fast, instead of a container per test run.
- **Prototype → production** — start embedded, then `veclite export` to `.vecdb` and graduate to the [Vectorizer](https://github.com/hivellm/vectorizer) server when you need the network.

## 📊 Performance

Measured on the pinned desktop reference profile — AMD Ryzen 9 7950X3D (16C/32T), 128 GB DDR5, Windows 10 Pro, Rust 1.96 release. Benches are Criterion with fixed-seed corpora, so runs are deterministic.

| Bench | Scale | Median |
|---|---:|---:|
| `search/top10` | 2 000 × 512, cosine | **≈ 0.92 ms** |
| `index_build` | 1 000 × 512, cosine | ≈ 810 ms |
| `batch_insert` | 500 × 512, cosine | ≈ 238 ms |

These are **smoke-scale** numbers. Search p50 is well inside the **< 3 ms** target at this scale; the 1M × 512 SQ-8 reference target is measured on the pinned cloud runner (AWS `c7a.4xlarge`) with the full nightly bench.

### Server parity (gate G1)

Against a pinned `hivehub/vectorizer:3.5.0` container, identical clustered 512-d corpus (1 000 vectors, 50 queries), cosine:

> **mean top-10 overlap = 0.9920 ≥ 0.99** ✓

The comparison has real discriminating power: a metric mismatch scores ≈ 0.57 and loosely-clustered data ≈ 0.97 — only correct, structurally clear data clears the 0.99 floor.

### Quality gates

| Gate | Floor |
|---|---|
| Tests | 447 passing, zero warnings (`-D warnings`) |
| Coverage — `veclite` core | ≥ 93 % |
| Coverage — `veclite-ffi` | ≥ 95 % |
| Crash suite | 10 000 randomized + kill-9 iterations nightly (Linux/macOS/Windows) |
| `.veclite` v1 format | golden-file byte comparison (frozen at G2) |

Methodology, hardware profiles, and the parity harness: [`docs/benchmarks.md`](docs/benchmarks.md). Coverage policy: [`docs/COVERAGE.md`](docs/COVERAGE.md).

## 🔄 Relationship to Vectorizer

| | Vectorizer | VecLite |
|---|---|---|
| Model | client-server | embedded library |
| Storage | managed `data/` dir | single `.veclite` file |
| Access | REST / RPC / gRPC / MCP | function calls |
| Scale | replication, Raft, sharding | one process |
| Use | shared production infra | CLIs, apps, agents, edge, tests |

Same math, same defaults, same concepts — start embedded, graduate to the server when you need the network. Quantization and SIMD kernels are vendored byte-identical from the server; embedding providers are pinned to server outputs by a parity corpus.

## 🗺️ Status & Roadmap

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
| 6a | Fuzz/soak/sanitizer hardening harnesses (72 h / 24 h evidence accumulating toward G6) | ✅ Complete |
| 6b–6c | Docs & benchmark report · registry publishing · release 1.0 | 📋 Planned |

**Legend**: ✅ Complete · 🔄 In progress · 📋 Planned

Scoped forward per the [DAG](docs/DAG.md): SIMD ISA backends (scalar oracle ships now) and `DotProduct` HNSW (served by exact brute force — blocked by the pinned `anndists` dot-bound).

## 📚 Documentation

### 📖 Getting Started
- [Vision and scope](docs/vectorizer-lite/01-vision-and-scope.md) — what's in, what's out, feature matrix vs Vectorizer
- [Core API](docs/vectorizer-lite/03-core-api.md) — the Rust API (source of truth for all bindings)
- [C ABI guide](docs/c-abi.md) — linking `veclite-ffi` from C/C++ and other languages

### 🔧 Implementation Contract
- [`docs/specs/`](docs/specs/README.md) — SPEC-001…016, the normative component specs
- [PRD](docs/PRD.md) — requirements, NFRs, and release criteria
- [DAG](docs/DAG.md) — task dependency graph

### 🏗️ Design Rationale
- [Architecture](docs/vectorizer-lite/02-architecture.md) — crate layout, reuse of `vectorizer-core`, design decisions
- [Storage format](docs/vectorizer-lite/04-storage-format.md) — single-file `.veclite`, WAL, crash safety
- [Embeddings](docs/vectorizer-lite/05-embeddings.md) — BYO vectors, built-in BM25/TF-IDF, optional ONNX
- [SDK bindings](docs/vectorizer-lite/06-sdk-bindings.md) — Python, Node.js, Go, C#, WASM (native, not network)
- [Vectorizer compatibility](docs/vectorizer-lite/07-vectorizer-compatibility.md) — the graduation path to the server
- [Roadmap](docs/vectorizer-lite/08-roadmap.md) — phases to 1.0

### 📊 Performance & Testing
- [Benchmarks & server parity](docs/benchmarks.md) — methodology, pinned hardware, gate G1
- [Coverage policy](docs/COVERAGE.md) — what's measured, what the floors are, and why
- [CHANGELOG](CHANGELOG.md) — Keep a Changelog format

## 🤝 Contributing

Contributions are welcome. This project follows the HiveLLM family conventions: spec-driven development (PRD → DAG → SPEC → tests), Conventional Commits, Keep a Changelog, and zero-warning quality gates.

1. Fork, branch (`git checkout -b feat/your-feature`).
2. Read the relevant spec in [`docs/specs/`](docs/specs/README.md) — the specs are normative, not descriptive.
3. Make changes + tests (the coverage floors above are enforced by `cargo xtask coverage`).
4. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`.
5. Conventional commits (`feat(scope): …`, `fix(scope): …`, `docs(scope): …`).
6. Submit a PR with description, tests, and doc updates.

This repo uses [@hivehub/rulebook](https://github.com/hivellm/rulebook) for spec-driven development — proposals and tasks live under [`.rulebook/tasks/`](.rulebook/tasks/), decisions under [`.rulebook/decisions/`](.rulebook/decisions/).

**Changes to the `.veclite` v1 byte format are not accepted** — the format is frozen at gate G2 and guarded by committed golden files.

## 🙏 Acknowledgments

- [Vectorizer](https://github.com/hivellm/vectorizer) — the server engine VecLite distributes in-process; quantization and SIMD kernels are vendored byte-identical from it
- [anndists](https://crates.io/crates/anndists) — HNSW distance bounds
- [fastembed](https://crates.io/crates/fastembed) — local ONNX sentence-transformer models behind the `onnx` feature
- [PyO3](https://pyo3.rs/) / [maturin](https://www.maturin.rs/) · [napi-rs](https://napi.rs/) · [wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/) · [cbindgen](https://github.com/mozilla/cbindgen) — the binding toolchains
- Inspired by [SQLite](https://sqlite.org/) — one linked library, one file, zero configuration

## 📞 Contact

- 🐛 Issues: [github.com/hivellm/veclite/issues](https://github.com/hivellm/veclite/issues)
- 💬 Discussions: [github.com/hivellm/veclite/discussions](https://github.com/hivellm/veclite/discussions)
- 📧 Email: team@hivellm.org

## 📄 License

Apache License 2.0 — see [LICENSE](LICENSE). Same as the rest of the HiveLLM family.

---

**Built with ❤️ in Rust Edition 2024** 🦀

_Part of the [HiveLLM](https://github.com/hivellm) family (Vectorizer · Nexus · Synap · Fluxum · Thunder)._

[⭐ Star on GitHub](https://github.com/hivellm/veclite) • [📖 Docs](docs/) • [🚀 Quick Start](#-quick-start)
