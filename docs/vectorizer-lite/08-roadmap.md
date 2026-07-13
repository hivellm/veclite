# 08 — Roadmap

## Phasing principles

- Each phase ships something usable and ends with quality gates green (fmt, clippy `-D warnings`, tests, crash-safety suite where applicable).
- Rust core first; bindings only after the core API stops churning (phase 4). The FFI layer is the API-freeze forcing function.
- Format v1 is frozen at the end of phase 2 — everything after reads/writes the same file.

## Phase 0 — Repo bootstrap

- Workspace skeleton (`crates/veclite`, CI: fmt + clippy + test on Linux/macOS/Windows).
- Depend on `vectorizer-core` (published); confirm quantization + SIMD kernels compile standalone on all targets incl. `wasm32-unknown-unknown`.
- Port `VecLiteError`, `DistanceMetric`, options structs with server-parity defaults.
- **Exit criterion**: `cargo test` green on 3 OS; `cargo build --target wasm32-unknown-unknown` compiles core skeleton.

## Phase 1 — Engine MVP (in-memory)

- Collection registry, CRUD (`upsert/upsert_batch/get/delete`), HNSW wrapper extracted from `db/optimized_hnsw.rs` (CPU path only, sync).
- `search` / `query` builder with `ef_search` override; SQ-8 quantization default; `VecLite::memory()`.
- Criterion benches vs targets (1 M × 512-dim, p50 < 3 ms).
- **Exit criterion**: search-quality parity harness — same data into VecLite and Vectorizer server, top-10 overlap ≥ 0.99.

## Phase 2 — Persistence (format v1)

- `.veclite` single-file storage: header/TOC/segments, WAL sidecar, recovery, checkpoint, mmap reads, file lock, `snapshot`, `vacuum`, `read_only`.
- Crash-kill + torn-write + bit-flip test suites ([04](04-storage-format.md) §test plan).
- **Exit criterion**: crash suite 10 000 iterations, zero corruption; format v1 frozen and documented.

## Phase 3 — Search depth

- Payload storage + payload indexes (keyword/int/float) + filter evaluation (must/should/must_not; eq/in/range/exists).
- Sparse vectors, BM25 auto-embed provider (+ tfidf/bow/char_ngram), vocabulary persistence + `refit()`, hybrid search with RRF, `search_text`, chunker utility, aliases, `scroll`, batch search.
- **Exit criterion**: filter + hybrid results conformance vs server on the shared corpus.

## Phase 4 — FFI + first bindings (API freeze)

- `veclite-ffi` C ABI + cbindgen header; `catch_unwind` hardening; error-code mapping.
- **Python** (PyO3, abi3, NumPy zero-copy, maturin wheels) and **Node** (napi-rs, prebuilds) — the two priority ecosystems.
- Binding conformance suite (YAML corpus) in CI on the full platform matrix.
- **Exit criterion**: `pip install veclite` / `npm install veclite` on clean machines (no Rust toolchain) runs the quickstart.

## Phase 5 — Ecosystem breadth

- Go (cgo) and C# (NuGet) over the C ABI; WASM package (in-memory + OPFS + serialize/deserialize).
- `onnx` feature + separate heavy packages (`veclite[onnx]`, `@veclite/onnx`, `VecLite.Onnx`).
- `vecdb-interop`: `veclite import/export` CLI, graduation-path round-trip test vs a live Vectorizer server.
- **Exit criterion**: graduation round-trip meets ≥ 0.99 overlap target; all 5 language quickstarts in docs are CI-executed.

## Phase 6 — 1.0 hardening

- Fuzzing (cargo-fuzz on format parser + WAL replay), 24 h soak (write/search/vacuum loop), memory-pressure tests (mmap datasets 4× RAM).
- Docs site, migration guide, benchmark report vs server & vs peer embedded stores (sqlite-vec, LanceDB embedded, Chroma embedded) — honest, reproducible harness.
- Format stability pledge: v1 readable by every future 1.x; semver policy published.
- **Exit criterion**: all [01 §success criteria](01-vision-and-scope.md) checked off.

## Post-1.0 candidates (explicitly deferred, in rough priority order)

| Candidate | Trigger to schedule |
|---|---|
| Shared `vectorizer-engine` crate (de-fork engine logic with the server) | VecLite API stable ≥ 2 minors; server team bandwidth |
| Graph relationships (edges/neighbors/paths) | user demand from the server feature's embedded users |
| Geo + nested filter conditions | first real request |
| Product quantization by default paths | memory-constrained mobile adopters |
| Change streams / update hooks | integration frameworks ask for reactivity |
| Encryption at rest (per-file) | regulated-industry embedded adopters |
| Multi-process readers of a live-written file (WAL-share, SQLite-style) | serverless platforms with concurrent instances |
| Mobile targets (iOS/Android via uniffi or C ABI) | after desktop matrix is boring |

## Versioning & release policy

- SemVer. 0.x during phases 0–5; 1.0.0 at phase 6 exit.
- File-format version is independent of crate version (format v1 spans many releases); `min_reader_version` gates forward-compat.
- Core + all bindings release in lockstep with one version number.
- CHANGELOG per Conventional Commits (`feat`/`fix`/`perf`/…), same discipline as the Vectorizer repo.

## Risks

| Risk | Mitigation |
|---|---|
| `hnsw_rs 0.3` serialization not stable across versions | pin exact version; graph segment carries its own version byte; rebuild-from-vectors fallback already designed in |
| `vectorizer-core` evolves server-first, breaks VecLite | pinned minor + conformance CI in both repos ([07 §divergence policy](07-vectorizer-compatibility.md)) |
| Windows mmap + truncate friction (vacuum) | pager designed for unmap→truncate→remap from day one; CI runs the crash suite on Windows |
| Prebuild matrix maintenance burden (5 ecosystems × 3 OS × 2 arch) | single GitHub Actions reusable workflow, napi-rs/maturin standard tooling; ship Python+Node first, others follow demand |
| Scope creep back toward the server (auth, watchers, replication requests) | non-goals in [01](01-vision-and-scope.md) are the contract; requests get redirected to the graduation path |
