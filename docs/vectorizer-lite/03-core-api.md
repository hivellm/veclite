# 03 — Core API (Rust)

The Rust API is the source of truth; every binding ([06-sdk-bindings.md](06-sdk-bindings.md)) mirrors it 1:1 with idiomatic naming. Design bias: **small surface, obvious defaults, everything synchronous**.

## Opening a database

```rust
use veclite::{VecLite, OpenOptions};

// Zero-config: creates the file if missing, WAL on, SQ-8 quantization default.
let db = VecLite::open("app.veclite")?;

// Tuned open:
let db = OpenOptions::new()
    .read_only(true)              // mmap the file, refuse writes, skip the lock
    .mmap(true)                   // default true for files > 64 MiB
    .background_checkpoint(false) // default false — no threads unless asked
    .open("app.veclite")?;

// Ephemeral (tests, agents):
let db = VecLite::memory();
```

`Database` (`VecLite` is the entry type name) is `Send + Sync + Clone` (internal `Arc`). Drop of the last handle flushes the WAL and releases the file lock.

## Collections

```rust
use veclite::{CollectionOptions, Metric, Quantization};

// Bring-your-own-vectors collection:
let docs = db.create_collection("docs",
    CollectionOptions::new(384, Metric::Cosine))?;

// Auto-embedding collection (text in, vectors managed internally):
let notes = db.create_collection("notes",
    CollectionOptions::auto_embed("bm25", 512))?;   // provider + dimension

// Full tuning:
let tuned = db.create_collection("tuned",
    CollectionOptions::new(768, Metric::DotProduct)
        .hnsw(/* m */ 16, /* ef_construction */ 200, /* ef_search */ 100)
        .quantization(Quantization::Scalar { bits: 8 })   // default
        .payload_index("lang", PayloadIndexKind::Keyword) // pre-declared filter index
        .payload_index("year", PayloadIndexKind::Integer))?;

let docs = db.collection("docs")?;        // get handle
db.rename_collection("docs", "docs_v2")?;
db.create_alias("docs", "docs_v2")?;      // aliases for blue/green reindex
db.delete_collection("old")?;
let names: Vec<String> = db.list_collections();
```

Defaults (server parity, `models/mod.rs` in Vectorizer):

| Option | Default | Server default |
|---|---|---|
| `metric` | `Cosine` | `Cosine` |
| `hnsw.m` | 16 | 16 |
| `hnsw.ef_construction` | 200 | 200 |
| `hnsw.ef_search` | 100 | 100 |
| `quantization` | `Scalar { bits: 8 }` | `SQ { bits: 8 }` |
| `compression` | LZ4, threshold 1 KiB | LZ4, threshold 1024 B |
| `embedding provider` (auto_embed) | `bm25` | `"bm25"` |

## Writing vectors

```rust
use veclite::{Point, Payload};

// Single + batch upsert (insert-or-replace; the embedded API has no separate insert/update)
docs.upsert(Point::new("id-1", vec![0.1; 384]).payload(json!({"lang": "en"})))?;
docs.upsert_batch(points)?;                       // Vec<Point>, one WAL entry

// Auto-embedding collections accept text:
notes.upsert_text("id-9", "the text to index", json!({"source": "readme"}))?;
notes.upsert_texts(vec![("id-10", "...", json!({}))])?;

docs.delete("id-1")?;
docs.delete_batch(&["id-2", "id-3"])?;
let p: Option<Point> = docs.get("id-1")?;
let n: usize = docs.len();
```

- IDs are strings (server parity). Numeric IDs pass through as their decimal form.
- `upsert` with a wrong dimension → `Err(DimensionMismatch { expected, got })` — never silent coercion (lesson from server issue #306).
- Payloads are JSON (`serde_json::Value`); binding layers expose native dicts/objects.

## Search

```rust
use veclite::{Filter, Condition};

// k-NN
let hits = docs.search(&query_vec, 10)?;               // Vec<Hit { id, score, payload }>

// With options
let hits = docs.query(&query_vec)
    .limit(10)
    .ef_search(200)                                     // per-query override
    .filter(Filter::must([
        Condition::eq("lang", "en"),
        Condition::range("year", 2020..=2026),
    ]))
    .with_payload(true)                                 // default true
    .with_vector(false)                                 // default false
    .run()?;

// Text query on auto-embedding collections
let hits = notes.search_text("how do I configure logging", 5)?;

// Hybrid: dense + sparse fused with RRF
let hits = docs.hybrid_query()
    .dense(&query_vec)
    .sparse(&sparse_query)          // SparseVector { indices, values }
    .alpha(0.5)                     // dense/sparse balance
    .limit(10)
    .run()?;

// Batch search (rayon-parallel internally)
let all: Vec<Vec<Hit>> = docs.search_batch(&queries, 10)?;

// Pagination / full scan
let page = docs.scroll(ScrollOptions::new().limit(100).offset_id("id-500"))?;
```

Filter semantics come from the server's Qdrant-style model (`must` / `should` / `must_not`; conditions: `eq`, `in`, `range`, `exists`) — geo and nested conditions deferred post-1.0.

## Maintenance

```rust
db.snapshot("backup-2026-07-12.veclite")?;  // consistent point-in-time single-file copy
db.vacuum()?;                                // compaction: reclaim deleted-vector space
db.checkpoint()?;                            // force WAL → main file
docs.reindex()?;                             // rebuild HNSW (e.g., after bulk delete)
let s: CollectionStats = docs.stats();       // count, memory, index size, quantization ratio
let i: DatabaseInfo = db.info();             // file size, format version, collections
```

## Errors

```rust
#[non_exhaustive]
#[derive(thiserror::Error, Debug)]
pub enum VecLiteError {
    #[error("collection not found: {0}")]      CollectionNotFound(String),
    #[error("vector not found: {0}")]          VectorNotFound(String),
    #[error("collection already exists: {0}")] AlreadyExists(String),
    #[error("dimension mismatch: expected {expected}, got {got}")]
                                                DimensionMismatch { expected: usize, got: usize },
    #[error("database is locked by another process")] Locked,
    #[error("file is corrupt: {0}")]            Corrupt(String),
    #[error("format version {found} newer than supported {supported}")]
                                                UnsupportedFormatVersion { found: u32, supported: u32 },
    #[error("unknown embedding provider: {requested}; available: {available:?}")]
                                                UnsupportedProvider { requested: String, available: Vec<String> },
    #[error("read-only database")]              ReadOnly,
    #[error(transparent)]                       Io(#[from] std::io::Error),
}
```

Stable variants; FFI maps them to integer codes + message ([06](06-sdk-bindings.md)). No panics across the public boundary — panics in internal invariants are caught at the FFI layer and surfaced as `Corrupt`/internal error codes.

## What the API deliberately does NOT have

- No async functions (bindings add them where the platform expects it).
- No connection strings/URLs — a path or `memory()`.
- No auth, users, keys, tenants.
- No server-style `insert` vs `update` distinction — `upsert` only.
- No config file. Every knob is a typed option with a default.
- No callbacks/hooks in v1 (change-stream is a post-1.0 candidate).

## Example: end-to-end (Rust)

```rust
use veclite::{VecLite, CollectionOptions};

fn main() -> Result<(), veclite::VecLiteError> {
    let db = VecLite::open("kb.veclite")?;
    let kb = match db.collection("kb") {
        Ok(c) => c,
        Err(_) => db.create_collection("kb", CollectionOptions::auto_embed("bm25", 512))?,
    };

    kb.upsert_text("doc-1", std::fs::read_to_string("README.md")?.as_str(),
                   serde_json::json!({ "path": "README.md" }))?;

    for hit in kb.search_text("installation steps", 5)? {
        println!("{:.3}  {}  {}", hit.score, hit.id, hit.payload["path"]);
    }
    Ok(())
}
```
