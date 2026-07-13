# VecLite — Product Requirements Document (PRD)

| | |
|---|---|
| **Product** | VecLite — embedded, in-process vector database ("SQLite for semantic search") |
| **Status** | Approved for implementation (derived from the planning set in [`docs/vectorizer-lite/`](vectorizer-lite/README.md)) |
| **Version** | 1.0 of this document, targeting product release 1.0.0 |
| **Date** | 2026-07-12 |
| **Source project** | [hivellm/vectorizer](https://github.com/hivellm/vectorizer) v3.5.0 |
| **Related** | [DAG.md](DAG.md) (implementation plan) · [SPEC index](specs/README.md) (normative specs) |

---

## 1. Problem statement

Vectorizer is a production vector database **server**: it requires a running process, ports (15002/15503), configuration, and network client SDKs. That is the right shape for shared infrastructure — and the wrong shape for a large class of consumers:

- **CLI tools and desktop apps** that need semantic search over local data without asking users to run Docker.
- **Agent runtimes and test suites** that need an ephemeral, disposable index per run.
- **Serverless / edge / WASM environments** where a companion server process cannot exist.
- **Notebooks and scripts** where "install a package, open a file" is the entire acceptable setup budget.

Today the smallest possible Vectorizer deployment is: server binary + config file + open port + client SDK + lifecycle management. For the audiences above this is prohibitive friction, and they either use a weaker embedded alternative or skip vector search entirely.

## 2. Product vision

**VecLite is to Vectorizer what SQLite is to client-server databases**: the same engine — HNSW search, quantization, hybrid dense+sparse retrieval, payload filtering — delivered as a library you link, storing everything in one file you own.

```python
import veclite

db = veclite.open("app.veclite")
docs = db.create_collection("docs", auto_embed="bm25", dimension=512)
docs.upsert_text("readme", open("README.md").read(), {"lang": "en"})
hits = docs.search_text("how do I configure logging", limit=5)
```

No server. No ports. No configuration. One file.

## 3. Target users and personas

| Persona | Context | What they need | Primary tier |
|---|---|---|---|
| **P1 — App developer** (Python/Node) | Ships a CLI, desktop, or SaaS-adjacent tool that needs local semantic search | `pip install` / `npm install` with no Rust toolchain; 5-line quickstart; single copyable file | Bindings (Python/Node) |
| **P2 — Agent/RAG framework author** | Builds LLM agent runtimes, needs per-session ephemeral or per-project persistent indexes | In-memory mode, fast open, BYO vectors, deterministic behavior in tests | Rust core + bindings |
| **P3 — Rust systems developer** | Embeds search in a Rust service or tool | `cargo add veclite`, sync API, no tokio imposed, small dependency tree | Rust core |
| **P4 — Edge/WASM developer** | Client-side search in browser extensions, offline apps, edge functions | wasm32 build, OPFS persistence, serialize/deserialize bytes | WASM package |
| **P5 — Existing Vectorizer user** | Runs the server in production, wants dev fixtures / offline slices / on-ramp | `.vecdb` import/export, identical concepts and defaults, graduation path both ways | Interop + CLI |

## 4. Goals

1. **G1 — Embed the real engine.** Reuse Vectorizer's core algorithms (HNSW via `hnsw_rs 0.3`, SIMD distance kernels, scalar/product/binary quantization, LZ4/zstd compression) via the shared `vectorizer-core` crate. Search quality and performance match the server for single-process workloads. Not a toy reimplementation.
2. **G2 — Single-file storage.** One `.veclite` file per database (plus a transient `-wal` sidecar during writes) containing collections, vectors, indexes, payloads, and embedding state. Copyable, versionable, streamable.
3. **G3 — Zero configuration, no runtime imposition.** Synchronous, thread-safe core; no tokio, no ports, no daemon, no config file. `open(path)` with sane defaults is a complete setup.
4. **G4 — Native SDKs.** Python, Node.js, Go, C#, and WASM bindings that link the compiled core directly (PyO3, napi-rs, C-ABI/cgo, P/Invoke, wasm-bindgen). Installation never requires a Rust toolchain.
5. **G5 — Small default footprint.** Default build is pure Rust: no ONNX Runtime, no model downloads, no protoc, no C++ linkage. Target < 10 MB compiled library overhead.
6. **G6 — Graduation path.** Data and concepts map 1:1 to the Vectorizer server. Export/import bridges `.veclite` ↔ `.vecdb`; collection configs, metrics, HNSW params, provider IDs, and vocabularies carry over unchanged.

## 5. Non-goals (contract — requests get redirected to the graduation path)

| Non-goal | Rationale |
|---|---|
| Server features: REST/gRPC/MCP/GraphQL, dashboard, remote access | Transport is the server's job; VecLite is a function call |
| Multi-process shared writes | Like SQLite: one writer process (advisory lock), multi-threaded within it |
| Auth, API keys, RBAC, multi-tenancy, quotas | Process boundary is the security boundary |
| Replication, Raft, sharding, HA | Distributed concerns belong to Vectorizer |
| Ingestion platform: file watchers, workspace discovery, document conversion | VecLite ships a minimal text chunker utility only |
| Remote embedding APIs (OpenAI et al.) in core | Callers do HTTP themselves and pass vectors in |
| Payload encryption at rest (v1) | Host app / OS concern; post-1.0 candidate if demanded |

## 6. Functional requirements

Requirement IDs are stable and referenced by the specs and the [DAG](DAG.md). Priority: **P0** = 1.0 blocker, **P1** = 1.0 target, **P2** = post-1.0 candidate.

### 6.1 Database lifecycle

| ID | Requirement | Priority |
|---|---|---|
| FR-01 | Open/create a database from a filesystem path with zero mandatory configuration (`VecLite::open`). | P0 |
| FR-02 | Pure in-memory database (`VecLite::memory()`) with the identical API, no file, no WAL. | P0 |
| FR-03 | Tuned open via `OpenOptions`: `read_only`, `mmap`, `durability`, `background_checkpoint`, `model_cache_dir`. | P0 |
| FR-04 | Advisory file lock: exclusive for read-write, shared for read-only; second writer fails fast with `Locked`. | P0 |
| FR-05 | `snapshot(path)` — consistent point-in-time compacted single-file copy without blocking writers. | P0 |
| FR-06 | `vacuum()` — compaction reclaiming tombstoned space, shrinking the file in place. | P0 |
| FR-07 | `checkpoint()` — explicit WAL → main-file transfer. | P0 |
| FR-08 | `info()` — file size, format version, collection list. | P1 |

### 6.2 Collections

| ID | Requirement | Priority |
|---|---|---|
| FR-10 | Create/get/delete/rename/list collections; names unique per database. | P0 |
| FR-11 | `CollectionOptions`: dimension, metric (Cosine/Euclidean/DotProduct), HNSW (m/ef_construction/ef_search), quantization, declared payload indexes, auto-embed provider. Server-parity defaults (see SPEC-004 §3). | P0 |
| FR-12 | Collection aliases (create/delete/resolve) for blue-green reindex. | P1 |
| FR-13 | Per-collection `stats()`: vector count, memory, index size, quantization ratio. | P1 |
| FR-14 | `reindex()` — rebuild the HNSW graph (e.g., after bulk deletes). | P0 |

### 6.3 Vector CRUD

| ID | Requirement | Priority |
|---|---|---|
| FR-20 | `upsert` / `upsert_batch` (insert-or-replace; no separate insert/update). Batch is one atomic WAL entry. | P0 |
| FR-21 | `get(id)`, `delete(id)`, `delete_batch(ids)`, `len()`. | P0 |
| FR-22 | String IDs (≤ 512 bytes UTF-8); numeric IDs pass through as decimal strings. | P0 |
| FR-23 | Dimension mismatch on write → typed error, never silent coercion (lesson from server issue #306). | P0 |
| FR-24 | JSON payload per vector (≤ 16 MiB compressed); bindings expose native dicts/objects. | P0 |
| FR-25 | `scroll` pagination over a collection (limit + offset-id cursor). | P1 |

### 6.4 Search

| ID | Requirement | Priority |
|---|---|---|
| FR-30 | k-NN search over HNSW with per-query `ef_search` override; results = `Hit { id, score, payload? , vector? }`. | P0 |
| FR-31 | Query builder: `limit`, `filter`, `with_payload` (default true), `with_vector` (default false). | P0 |
| FR-32 | Payload filters: `must` / `should` / `must_not` with `eq`, `in`, `range`, `exists` conditions (Qdrant-model subset; geo/nested deferred P2). | P0 |
| FR-33 | Payload indexes: keyword, integer, float — declared at creation or added later. | P0 |
| FR-34 | Hybrid search: dense + sparse lanes fused with RRF, `alpha` balance parameter. | P0 |
| FR-35 | `search_batch` — rayon-parallel multi-query. | P1 |
| FR-36 | `search_text` on auto-embed collections (embed query + search in one call). | P0 |
| FR-37 | Optional search explain/trace behind the `explain` feature. | P2 |

### 6.5 Embeddings

| ID | Requirement | Priority |
|---|---|---|
| FR-40 | Bring-your-own-vectors is the primary, zero-machinery path. | P0 |
| FR-41 | Built-in pure-Rust providers: `bm25` (default), `tfidf`, `bow`, `char_ngram`; `svd` behind a feature. | P0 |
| FR-42 | Auto-embed collections: `upsert_text(s)` / `search_text`; provider + vocabulary state persisted inside the file; a `.veclite` file is fully self-contained. | P0 |
| FR-43 | Unknown provider → `UnsupportedProvider` listing available ones; never silent fallback. | P0 |
| FR-44 | `refit()` — explicit vocabulary recomputation + re-embed (never automatic). | P1 |
| FR-45 | Custom in-process providers via `register_embedder` (per-Database, never global). | P1 |
| FR-46 | Optional dense neural embeddings behind the `onnx` feature (`fastembed:<model>`), shipped as separate heavy packages; air-gapped local-path model loading. | P1 |
| FR-47 | Text chunker utility (UTF-8-safe, sentence/word-boundary, configurable size/overlap). | P1 |

### 6.6 Storage & durability

| ID | Requirement | Priority |
|---|---|---|
| FR-50 | Single-file `.veclite` format v1: header + append-ordered immutable segments + TOC, per-segment crc32, versioned with `min_reader_version`. | P0 |
| FR-51 | WAL sidecar with crash recovery: `kill -9` at any moment never corrupts the main file; WAL replay applies whole entries or discards torn tails. | P0 |
| FR-52 | Durability modes: `Full` (fsync per commit), `Normal` (fsync on checkpoint/close, default), `Off`. | P0 |
| FR-53 | mmap read path for larger-than-RAM datasets; fixed-stride vector segments readable without decode. | P0 |
| FR-54 | HNSW graph persisted in-file; corrupt/missing graph segment → rebuild-from-vectors fallback with warning. | P0 |
| FR-55 | LZ4 (default, threshold 1 KiB) / zstd block compression. | P0 |
| FR-56 | Forward/backward compat: newer readers accept all older format versions; older readers fail with `UnsupportedFormatVersion`. | P0 |

### 6.7 Bindings & distribution

| ID | Requirement | Priority |
|---|---|---|
| FR-60 | C ABI (`veclite-ffi`): handle-based, error codes 1:1 with `VecLiteError`, `catch_unwind` at every entry, cbindgen header, additive-only within a major. | P0 |
| FR-61 | Python binding: abi3 wheels, NumPy zero-copy, GIL released around core calls, context managers, optional asyncio facade. | P0 |
| FR-62 | Node binding: napi-rs, async-by-default + `*Sync` twins, `Float32Array` zero-copy, per-platform prebuild packages. | P0 |
| FR-63 | Go binding (cgo) and C# binding (P/Invoke, `SafeHandle`, `Span<T>`) over the C ABI. | P1 |
| FR-64 | WASM package: in-memory + OPFS persistence + `serialize`/`deserialize` bytes; simd128; no threads/mmap/onnx. | P1 |
| FR-65 | Binding conformance suite: one YAML corpus executed by every binding's CI; release-blocking. | P0 |
| FR-66 | Prebuilt matrix: Linux (glibc+musl)/macOS/Windows × x64/arm64 for every package; install never requires a Rust toolchain. | P0 |

### 6.8 Vectorizer interop

| ID | Requirement | Priority |
|---|---|---|
| FR-70 | `vecdb-interop` feature: export `.veclite` → `.vecdb` + `.vecidx` the server imports; import both server layouts (Compact + Legacy). | P1 |
| FR-71 | `veclite` CLI: `export`, `import` (with `--collections` selection), `inspect`. | P1 |
| FR-72 | Import drops server-only aspects with warnings (tenants, shards merged, graph edges); refuses encrypted payloads; server-only-provider collections import as BYO with recorded `origin_provider`. | P1 |
| FR-73 | Shared conformance corpus run in both repos' CI; behavior divergence = bug (documented exceptions only). | P1 |

## 7. Non-functional requirements

| ID | Requirement | Measure |
|---|---|---|
| NFR-01 | **Search latency**: 1 M × 512-dim vectors, p50 search < 3 ms (default HNSW params, SQ-8, warm). | Criterion bench in CI |
| NFR-02 | **Open time**: mmap warm open of the 1 M-vector file < 100 ms. | Bench |
| NFR-03 | **Index build**: within 2× of Vectorizer server single-node build time on the same data. | Bench vs server |
| NFR-04 | **Search parity**: top-10 overlap ≥ 0.99 vs the server on the standard benchmark set (same data/config). | Parity harness |
| NFR-05 | **Crash safety**: 10 000-iteration kill/torn-write/bit-flip suite with zero main-file corruption. | Crash suite |
| NFR-06 | **Footprint**: default build < 10 MB linked-library overhead; clean compile < 60 s on a laptop. | CI check |
| NFR-07 | **No background threads by default**; opt-in only (`background_checkpoint`). | Code review + test |
| NFR-08 | **No network I/O in core, ever.** ONNX model download only on explicit provider construction, redirectable to local path. | Code review + deny-list test |
| NFR-09 | **No panics across public boundaries**; `unwrap_used = "deny"` lint policy. | Clippy CI |
| NFR-10 | **Thread safety**: `Database`/`Collection` are `Send + Sync`; concurrent read/write soak passes under sanitizers. | Loom/sanitizer CI |
| NFR-11 | **Format stability**: v1 files readable by every future 1.x release (pledge published at 1.0). | Compat test corpus |
| NFR-12 | **API stability**: SemVer; FFI ABI additive-only within a major; core + bindings release in lockstep. | Release policy |

## 8. Competitive landscape (positioning, not a battle plan)

| Alternative | VecLite differentiation |
|---|---|
| **sqlite-vec** | Full HNSW ANN (not brute force), quantization, hybrid search, native multi-language bindings |
| **LanceDB embedded** | Single file (not a directory), no async runtime imposed, smaller default footprint, server graduation path |
| **Chroma embedded** | In-process native core (not a Python-first client), Rust/Go/C#/WASM reach, crash-safe single file |
| **FAISS / hnswlib** | A database (persistence, payloads, filters, hybrid), not just an index structure |
| **Vectorizer server** | Not a competitor: the graduation target; same math, same defaults, documented both-way migration |

The 1.0 benchmark report (DAG task T6.4) publishes honest, reproducible comparisons against these.

## 9. Release criteria (1.0.0 go/no-go)

All of the following, verified by CI or a documented manual protocol:

1. ✅ `cargo add veclite` → open/insert/search with zero config; NFR-06 footprint targets met.
2. ✅ `pip install veclite` and `npm install veclite` on clean machines (no Rust toolchain) across the FR-66 matrix run the quickstart.
3. ✅ NFR-01/02/03 performance targets green on the reference hardware profile.
4. ✅ NFR-05 crash suite: 10 000 iterations, zero corruption.
5. ✅ Graduation round-trip: `veclite export --vecdb` → server import → top-10 overlap ≥ 0.99 (NFR-04 corpus).
6. ✅ Binding conformance suite green for Python, Node, Go, C#, WASM on the full platform matrix.
7. ✅ Format v1 frozen, documented (SPEC-002), stability pledge published.
8. ✅ Docs site with all 5 language quickstarts CI-executed; migration guide (both directions) published.

## 10. Success metrics (post-launch, first 6 months)

| Metric | Target |
|---|---|
| Package installs (PyPI + npm combined) | Baseline established; growth trend positive month-over-month |
| Time-to-first-search in user telemetry-free proxy (docs quickstart completion issues) | < 5 min reported path; zero "couldn't install" P0 issues open > 1 week |
| Graduation conversions (VecLite → Vectorizer server, self-reported/issues) | ≥ 3 documented cases validating the funnel thesis |
| Corruption reports against the frozen format | 0 confirmed |
| Cross-repo conformance divergences discovered post-release | 0 undocumented |

## 11. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| `hnsw_rs 0.3` serialization instability across versions | Broken graph segments on upgrade | Pin exact version; graph segment carries its own version byte; rebuild-from-vectors fallback (FR-54) |
| `vectorizer-core` evolves server-first and breaks VecLite | Parity drift, blocked releases | Pin minor line; conformance CI in both repos; changes to math/encodings must land in `vectorizer-core` first (shared-crate policy) |
| Windows mmap + truncate friction (vacuum) | Corruption or failures on Windows | Pager designed for unmap→truncate→remap; crash suite runs on Windows CI |
| Prebuild matrix burden (5 ecosystems × 3 OS × 2 arch) | Release grind, stale platforms | One reusable GH Actions workflow; standard maturin/napi-rs tooling; Python+Node first, others follow demand |
| Scope creep back toward server features | Bloat, delayed 1.0 | §5 non-goals are the contract; requests redirect to graduation path |
| API churn after bindings exist | Costly multi-repo breaking changes | Bindings start only at Phase 4; FFI layer is the API-freeze forcing function |

## 12. Open questions

Tracked here until resolved; resolution updates the relevant SPEC.

| # | Question | Owner decision needed by |
|---|---|---|
| OQ-1 | Exact reference hardware profile for NFR-01/02/03 benchmarks (pin a cloud instance type + a laptop class). | Phase 1 exit (T1.6) |
| OQ-2 | Minimum supported Rust version (MSRV) policy — track `vectorizer-core`'s or pin independently? | Phase 0 (T0.1) |
| OQ-3 | WASM OPFS shim design: sync-core-over-async-storage needs a buffering strategy — full-file buffer vs block cache. | Phase 5 start (T5.3) |
| OQ-4 | CLI distribution: separate `veclite-cli` crate/binary or feature of the core crate? | Phase 5 start (T5.5) |
| OQ-5 | Whether `bincode 2` or MessagePack is used for CONFIG segments (planning says bincode 2; conformance with FFI payload codec favors one codec everywhere). | Phase 2 start (T2.2) |

## 13. Document map

| Document | Role |
|---|---|
| [PRD.md](PRD.md) (this) | What and why; requirement IDs; release criteria |
| [DAG.md](DAG.md) | Implementation dependency graph; task breakdown mapped to phases and specs |
| [SPEC-001 … SPEC-016](specs/README.md) | How, normatively — each spec traces back to FR/NFR IDs here |
| [`docs/vectorizer-lite/`](vectorizer-lite/README.md) | Original planning set (design rationale, source-code mapping to Vectorizer) |
