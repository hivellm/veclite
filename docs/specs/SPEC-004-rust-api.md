# SPEC-004 — Rust API (source of truth for all bindings)

| | |
|---|---|
| **Status** | Draft — **frozen at T4.1** (FFI extraction is the freeze event); additive-only afterwards within 1.x |
| **Phase / tasks** | Phases 0–3 · T0.3, T1.4, T1.5, T3.10 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-01–08, FR-10–14, FR-20–25, FR-30–37; NFR-12 |
| **Planning source** | [03-core-api.md](../vectorizer-lite/03-core-api.md) |

Requirement IDs `API-xxx`. Every binding (SPEC-009…012) MUST mirror this surface 1:1 with idiomatic naming; behavioral parity is enforced by the conformance corpus (SPEC-015 §3).

## 1. Entry points

```rust
pub struct VecLite;                          // a.k.a. Database handle
impl VecLite {
    pub fn open(path: impl AsRef<Path>) -> Result<VecLite>;   // zero-config
    pub fn memory() -> VecLite;                                // no file, no WAL
}

pub struct OpenOptions;                       // builder
impl OpenOptions {
    pub fn new() -> Self;
    pub fn read_only(self, v: bool) -> Self;              // default false
    pub fn mmap(self, v: bool) -> Self;                   // default: auto (files > 64 MiB)
    pub fn durability(self, v: Durability) -> Self;       // default Normal
    pub fn background_checkpoint(self, v: bool) -> Self;  // default false
    pub fn wal_size_limit(self, bytes: u64) -> Self;      // default 64 MiB
    pub fn auto_vacuum_threshold(self, ratio: f32) -> Self; // default 0.25
    pub fn read_only_ignore_wal(self, v: bool) -> Self;   // default false (SPEC-003 WAL-043)
    pub fn model_cache_dir(self, path: impl AsRef<Path>) -> Self; // onnx feature
    pub fn on_warning(self, cb: impl Fn(Warning) + Send + Sync + 'static) -> Self;
    pub fn open(self, path: impl AsRef<Path>) -> Result<VecLite>;
}
```

- **API-001** `VecLite` is `Send + Sync + Clone` (internal `Arc`); drop of the last handle checkpoints, sets clean-close, releases the lock (SPEC-003 §7).
- **API-002** `open` with defaults MUST be a complete setup: creates the file if missing, WAL on, `Durability::Normal`, SQ-8 default quantization — no other call required before use (FR-01).
- **API-003** No connection strings, URLs, or config files anywhere in the API. Every knob is a typed option with a documented default.
- **API-004** `Warning` covers non-fatal conditions: HNSW rebuild fallback (STG-063), stale WAL ignored (WAL-002), auto-vacuum triggered. The callback MUST NOT be invoked after `close`.

## 2. Database methods

```rust
impl VecLite {
    pub fn create_collection(&self, name: &str, opts: CollectionOptions) -> Result<Collection>;
    pub fn collection(&self, name: &str) -> Result<Collection>;       // resolves aliases
    pub fn delete_collection(&self, name: &str) -> Result<()>;
    pub fn rename_collection(&self, from: &str, to: &str) -> Result<()>;
    pub fn create_alias(&self, alias: &str, target: &str) -> Result<()>;
    pub fn delete_alias(&self, alias: &str) -> Result<()>;
    pub fn list_collections(&self) -> Vec<String>;
    pub fn snapshot(&self, path: impl AsRef<Path>) -> Result<()>;
    pub fn vacuum(&self) -> Result<()>;
    pub fn checkpoint(&self) -> Result<()>;
    pub fn info(&self) -> DatabaseInfo;   // file size, format version, collections, wal size
    pub fn close(self) -> Result<()>;     // explicit, idempotent via handle count
    pub fn register_embedder(&self, name: &str, e: Box<dyn Embedder>) -> Result<()>; // SPEC-005
}
```

## 3. CollectionOptions and defaults

```rust
pub struct CollectionOptions;
impl CollectionOptions {
    pub fn new(dimension: usize, metric: Metric) -> Self;
    pub fn auto_embed(provider: &str, dimension: usize) -> Self;      // SPEC-005
    pub fn hnsw(self, m: usize, ef_construction: usize, ef_search: usize) -> Self;
    pub fn quantization(self, q: Quantization) -> Self;
    pub fn payload_index(self, key: &str, kind: PayloadIndexKind) -> Self;  // repeatable
}
pub enum Metric { Cosine, Euclidean, DotProduct }
pub enum Quantization { None, Scalar { bits: u8 }, Binary, #[cfg(feature="pq")] Product { .. } }
pub enum PayloadIndexKind { Keyword, Integer, Float }
```

**Defaults (server parity — normative; the conformance corpus pins them):**

| Option | Default |
|---|---|
| `metric` | `Cosine` |
| `hnsw.m` / `ef_construction` / `ef_search` | 16 / 200 / 100 |
| `quantization` | `Scalar { bits: 8 }` |
| `compression` | LZ4, threshold 1024 B |
| auto-embed provider | `"bm25"` |
| `with_payload` / `with_vector` (search) | `true` / `false` |

- **API-010** Changing a default is a breaking change (major version) — bindings and the server contract depend on them.

## 4. Collection methods

```rust
impl Collection {
    // writes (upsert-only model; FR-20)
    pub fn upsert(&self, point: Point) -> Result<()>;
    pub fn upsert_batch(&self, points: Vec<Point>) -> Result<()>;          // one WAL entry
    pub fn upsert_text(&self, id: &str, text: &str, payload: impl Into<Option<Value>>) -> Result<()>;
    pub fn upsert_texts(&self, items: Vec<(String, String, Option<Value>)>) -> Result<()>;
    pub fn delete(&self, id: &str) -> Result<bool>;                        // false = absent
    pub fn delete_batch(&self, ids: &[&str]) -> Result<usize>;             // count deleted

    // reads
    pub fn get(&self, id: &str) -> Result<Option<Point>>;
    pub fn len(&self) -> usize;   pub fn is_empty(&self) -> bool;
    pub fn scroll(&self, opts: ScrollOptions) -> Result<Page>;             // FR-25

    // search
    pub fn search(&self, vector: &[f32], limit: usize) -> Result<Vec<Hit>>;
    pub fn query(&self, vector: &[f32]) -> QueryBuilder;                   // §5
    pub fn search_text(&self, text: &str, limit: usize) -> Result<Vec<Hit>>;
    pub fn hybrid_query(&self) -> HybridQueryBuilder;                      // SPEC-007
    pub fn search_batch(&self, queries: &[Vec<f32>], limit: usize) -> Result<Vec<Vec<Hit>>>;

    // maintenance
    pub fn reindex(&self) -> Result<()>;
    pub fn refit(&self) -> Result<()>;                                     // SPEC-005 §5
    pub fn create_payload_index(&self, key: &str, kind: PayloadIndexKind) -> Result<()>;
    pub fn stats(&self) -> CollectionStats;
}
```

- **API-020** `upsert` = insert-or-replace; there is deliberately no separate insert/update (matches planning; simplifies bindings and the WAL model).
- **API-021** `delete`/`get` on missing IDs are not errors (`false`/`None`); errors are reserved for misuse and environment failures.
- **API-022** `scroll` is cursor-based: `ScrollOptions::new().limit(n).offset_id(id)`; ordering is stable by slot (insertion order after last vacuum). `Page { points, next_offset_id: Option<String> }`.

## 5. Query builder

```rust
docs.query(&vec)
    .limit(10)                 // default 10
    .ef_search(200)            // per-query override (CORE-031 bounds)
    .filter(Filter::must([Condition::eq("lang","en"), Condition::range("year", 2020..=2026)]))
    .with_payload(true)        // default true
    .with_vector(false)        // default false
    .run()?;
```

- **API-030** Builders are plain data until `.run()`; they hold no locks. `Filter`/`Condition` semantics are SPEC-006; `HybridQueryBuilder` is SPEC-007.
- **API-031** `limit = 0` → `InvalidArgument`. `limit` > live count returns all live vectors, no error.

## 6. Errors

```rust
#[non_exhaustive]
#[derive(thiserror::Error, Debug)]
pub enum VecLiteError {
    CollectionNotFound(String),   VectorNotFound(String),   AlreadyExists(String),
    DimensionMismatch { expected: usize, got: usize },
    Locked,   WalPending,   ReadOnly,   Closed,
    Corrupt(String),
    UnsupportedFormatVersion { found: u32, supported: u32 },
    UnsupportedProvider { requested: String, available: Vec<String> },
    InvalidArgument(String),
    Io(#[from] std::io::Error),
}
```

- **API-040** The enum is `#[non_exhaustive]`; **existing variants and their FFI codes (SPEC-008 §3) never change meaning within a major**. New variants may be added in minors.
- **API-041** No panics across the public boundary in safe usage (CORE-060/061).

## 7. Feature flags

```toml
[features]
default = ["simd"]
simd    = []             # vectorizer-core SIMD kernels
onnx    = ["dep:fastembed"]
pq      = []
svd     = ["dep:ndarray"]
cache   = []             # LRU query cache (off by default)
explain = []             # search tracing (FR-37)
vecdb-interop = []       # SPEC-013
```

- **API-050** The default build MUST NOT download anything at build or run time, link C++, or require protoc (planning rule; NFR-08).
- **API-051** Data written by feature-gated code MUST remain readable without the feature where possible (e.g., vectors of an `onnx` collection are searchable without `onnx`; only text re-embedding needs it — SPEC-005 §6).

## 8. API evolution rules

- **API-060** Pre-freeze (before T4.1): anything may change; no deprecation process.
- **API-061** Post-freeze, within 1.x: additive only — new methods, new builder options with defaults, new error variants, new feature flags. Removals/renames/semantic changes require 2.0.
- **API-062** Every public item carries rustdoc with at least one example; `#[deny(missing_docs)]` on the crate from Phase 4.

## 9. Acceptance criteria

1. Defaults table pinned by unit tests (T0.3) and by the conformance corpus (SPEC-015).
2. The end-to-end quickstart from [03-core-api.md §example](../vectorizer-lite/03-core-api.md) compiles and runs as a doctest.
3. `cargo public-api` (or equivalent) snapshot committed at T4.1; CI fails on non-additive changes afterwards.
4. In-memory and file-backed databases pass the identical behavioral corpus (API parity of FR-02).
