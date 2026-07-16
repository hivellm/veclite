//! Node.js binding for VecLite (SPEC-010), via napi-rs. Binds the Rust crate
//! directly (not the C ABI). The surface mirrors SPEC-004 method-for-method in
//! camelCase (NODE-010); every heavy operation has an async form that runs off
//! the JS thread on the tokio blocking pool (NODE-011) and a `*Sync` twin.
//!
//! Errors reject/throw a `VecLiteError` with a `code` string and the Rust
//! display message (NODE-020); the code is carried in the napi error reason as
//! `"<CODE>\u{1}<message>"` and split by the hand-written `index.js` loader.

#![deny(clippy::all)]

use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value;
use veclite::chunk::{ChunkOptions, Chunker};
use veclite::{
    CollectionOptions, HnswOptions, Metric, OpenOptions, PayloadIndexKind, Point, Quantization,
    SparseVector, VecLite, VecLiteError,
};

/// Map a `VecLiteError` to the JS error `code` string (NODE-020 — the FFI
/// constants without the `VL_ERR_` prefix).
fn code_of(e: &VecLiteError) -> &'static str {
    match e {
        VecLiteError::CollectionNotFound(_) => "COLLECTION_NOT_FOUND",
        VecLiteError::VectorNotFound(_) => "VECTOR_NOT_FOUND",
        VecLiteError::AlreadyExists(_) => "ALREADY_EXISTS",
        VecLiteError::DimensionMismatch { .. } => "DIMENSION_MISMATCH",
        VecLiteError::Locked => "LOCKED",
        VecLiteError::Corrupt(_) => "CORRUPT",
        VecLiteError::UnsupportedFormatVersion { .. } => "UNSUPPORTED_FORMAT_VERSION",
        VecLiteError::UnsupportedProvider { .. } => "UNSUPPORTED_PROVIDER",
        VecLiteError::ReadOnly => "READ_ONLY",
        VecLiteError::InvalidArgument(_) => "INVALID_ARGUMENT",
        VecLiteError::Io(_) => "IO",
        VecLiteError::WalPending => "WAL_PENDING",
        VecLiteError::Closed => "CLOSED",
        // `VecLiteError` is #[non_exhaustive]; a future variant maps to a
        // generic code until this table is extended.
        _ => "ERROR",
    }
}

/// Convert a `VecLiteError` into a napi error carrying `code` + message.
fn js_err(e: VecLiteError) -> Error {
    Error::new(Status::GenericFailure, format!("{}\u{1}{e}", code_of(&e)))
}

/// A napi error for a binding-layer (non-VecLite) failure.
fn arg_err(msg: impl Into<String>) -> Error {
    Error::new(
        Status::GenericFailure,
        format!("INVALID_ARGUMENT\u{1}{}", msg.into()),
    )
}

// ── Options objects (mirror SPEC-004; absent fields take the defaults) ───────

/// `open` / `openSync` options (mirrors `OpenOptions`, NODE-010).
#[napi(object)]
#[derive(Default)]
pub struct OpenOpts {
    pub read_only: Option<bool>,
    /// `"full" | "normal" | "off"`.
    pub durability: Option<String>,
    /// Force mmap on/off (default: auto over 64 MiB).
    pub mmap: Option<bool>,
    /// In-RAM index budget in bytes before a mapped collection scans (STG-064).
    pub memory_budget: Option<i64>,
}

/// HNSW tuning (mirrors `HnswOptions`).
#[napi(object)]
pub struct HnswOpts {
    pub m: Option<u32>,
    pub ef_construction: Option<u32>,
    pub ef_search: Option<u32>,
}

/// `createCollection` options (mirrors `CollectionOptions`).
#[napi(object)]
pub struct CollectionOpts {
    pub dimension: u32,
    /// `"cosine" | "euclidean" | "dotproduct"` (default cosine).
    pub metric: Option<String>,
    /// Scalar-quantization bit depth; 0 or absent = no quantization.
    pub quantization_bits: Option<u32>,
    pub hnsw: Option<HnswOpts>,
    /// Auto-embed provider id (e.g. `"bm25"`); absent = BYO vectors.
    pub auto_embed: Option<String>,
    /// Declared payload indexes as `[key, "keyword"|"integer"|"float"]` pairs.
    pub payload_indexes: Option<Vec<Vec<String>>>,
}

/// `search` / `searchSync` options.
#[napi(object)]
#[derive(Default)]
pub struct SearchOpts {
    pub limit: Option<u32>,
    pub ef_search: Option<u32>,
    pub with_payload: Option<bool>,
    pub with_vector: Option<bool>,
    /// A Qdrant-style filter document (SPEC-006), as a plain JS object.
    pub filter: Option<Value>,
}

/// One search hit projected to JS (NODE-011).
#[napi(object)]
pub struct JsHit {
    pub id: String,
    pub score: f64,
    pub payload: Option<Value>,
    pub vector: Option<Float32Array>,
}

/// A point for `upsertBatch` (BYO vectors): id + vector + optional payload and
/// optional `{indices, values}` sparse lane (SPEC-007).
#[napi(object)]
pub struct JsPoint {
    pub id: String,
    pub vector: Float32Array,
    pub payload: Option<Value>,
    pub sparse: Option<Value>,
}

fn metric_of(s: &Option<String>) -> Result<Metric> {
    Ok(match s.as_deref() {
        None | Some("cosine") => Metric::Cosine,
        Some("euclidean") => Metric::Euclidean,
        Some("dotproduct") => Metric::DotProduct,
        Some(other) => return Err(arg_err(format!("unknown metric {other:?}"))),
    })
}

fn payload_index_kind(s: &str) -> Result<PayloadIndexKind> {
    Ok(match s {
        "keyword" => PayloadIndexKind::Keyword,
        "integer" => PayloadIndexKind::Integer,
        "float" => PayloadIndexKind::Float,
        other => return Err(arg_err(format!("unknown payload index kind {other:?}"))),
    })
}

fn build_collection_options(o: &CollectionOpts) -> Result<CollectionOptions> {
    let mut opts = match &o.auto_embed {
        Some(provider) => CollectionOptions::auto_embed(provider, o.dimension as usize),
        None => CollectionOptions::new(o.dimension as usize, metric_of(&o.metric)?),
    };
    if let Some(bits) = o.quantization_bits {
        opts = opts.quantization(if bits == 0 {
            Quantization::None
        } else {
            Quantization::Scalar { bits: bits as u8 }
        });
    }
    if let Some(h) = &o.hnsw {
        let d = HnswOptions::default();
        opts = opts.hnsw(
            h.m.map_or(d.m, |v| v as usize),
            h.ef_construction.map_or(d.ef_construction, |v| v as usize),
            h.ef_search.map_or(d.ef_search, |v| v as usize),
        );
    }
    if let Some(indexes) = &o.payload_indexes {
        for pair in indexes {
            let (key, kind) = (
                pair.first()
                    .ok_or_else(|| arg_err("payload index needs a key"))?,
                pair.get(1)
                    .ok_or_else(|| arg_err("payload index needs a kind"))?,
            );
            opts = opts.payload_index(key, payload_index_kind(kind)?);
        }
    }
    Ok(opts)
}

fn build_open_options(o: &OpenOpts) -> Result<OpenOptions> {
    let mut opts = OpenOptions::new();
    if let Some(ro) = o.read_only {
        opts = opts.read_only(ro);
    }
    if let Some(d) = &o.durability {
        opts = opts.durability(match d.as_str() {
            "full" => veclite::Durability::Full,
            "normal" => veclite::Durability::Normal,
            "off" => veclite::Durability::Off,
            other => return Err(arg_err(format!("unknown durability {other:?}"))),
        });
    }
    if let Some(m) = o.mmap {
        opts = opts.mmap(m);
    }
    if let Some(b) = o.memory_budget {
        opts = opts.memory_budget(b.max(0) as u64);
    }
    Ok(opts)
}

/// Project ranked hits to JS, moving stored vectors into fresh `Float32Array`s.
fn project(hits: Vec<veclite::Hit>) -> Vec<JsHit> {
    hits.into_iter()
        .map(|h| JsHit {
            id: h.id,
            score: f64::from(h.score),
            payload: h.payload,
            vector: h.vector.map(Float32Array::new),
        })
        .collect()
}

fn sparse_from_value(v: &Value) -> Result<SparseVector> {
    serde_json::from_value(v.clone()).map_err(|e| arg_err(format!("invalid sparse vector: {e}")))
}

// ── Database ────────────────────────────────────────────────────────────────

/// A VecLite database handle (SPEC-004 §1). `VecLite` is internally `Arc`, so
/// cloning it into blocking tasks is cheap. The handle sits behind an
/// `Option` so `close()` can drop it deterministically (NODE-013) — releasing
/// the advisory file lock and running the close-time checkpoint — rather than
/// waiting for JS garbage collection.
#[napi]
pub struct Database {
    inner: std::sync::Mutex<Option<VecLite>>,
}

impl Database {
    fn wrap(db: VecLite) -> Self {
        Database {
            inner: std::sync::Mutex::new(Some(db)),
        }
    }

    /// A cloned handle for the current operation, or `Closed` after `close()`.
    fn handle(&self) -> Result<VecLite> {
        self.inner
            .lock()
            .map_err(|_| arg_err("database lock poisoned"))?
            .clone()
            .ok_or_else(|| js_err(VecLiteError::Closed))
    }
}

/// Open an ephemeral in-memory database (FR-02) — no file, identical API.
#[napi]
pub fn memory() -> Database {
    Database::wrap(VecLite::memory())
}

/// Open (or create) a file-backed database off the event loop (NODE-011).
#[napi]
pub async fn open(path: String, options: Option<OpenOpts>) -> Result<Database> {
    let opts = build_open_options(&options.unwrap_or_default())?;
    let inner = tokio::task::spawn_blocking(move || VecLite::open_with(path, opts))
        .await
        .map_err(|e| arg_err(e.to_string()))?
        .map_err(js_err)?;
    Ok(Database::wrap(inner))
}

/// Synchronous [`open`] for CLIs/scripts.
#[napi]
pub fn open_sync(path: String, options: Option<OpenOpts>) -> Result<Database> {
    let opts = build_open_options(&options.unwrap_or_default())?;
    Ok(Database::wrap(
        VecLite::open_with(path, opts).map_err(js_err)?,
    ))
}

#[napi]
impl Database {
    /// Create a collection (CORE-020). Async: runs off the event loop.
    #[napi]
    pub async fn create_collection(
        &self,
        name: String,
        options: CollectionOpts,
    ) -> Result<Collection> {
        let opts = build_collection_options(&options)?;
        let db = self.handle()?;
        let inner = tokio::task::spawn_blocking(move || db.create_collection(&name, opts))
            .await
            .map_err(|e| arg_err(e.to_string()))?
            .map_err(js_err)?;
        Ok(Collection { inner })
    }

    #[napi]
    pub fn create_collection_sync(
        &self,
        name: String,
        options: CollectionOpts,
    ) -> Result<Collection> {
        let opts = build_collection_options(&options)?;
        Ok(Collection {
            inner: self
                .handle()?
                .create_collection(&name, opts)
                .map_err(js_err)?,
        })
    }

    /// Get a collection handle by name or alias (CORE-051).
    #[napi]
    pub fn collection(&self, name: String) -> Result<Collection> {
        Ok(Collection {
            inner: self.handle()?.collection(&name).map_err(js_err)?,
        })
    }

    /// List collection names.
    #[napi]
    pub fn list_collections(&self) -> Result<Vec<String>> {
        Ok(self.handle()?.list_collections())
    }

    /// Delete a collection (CORE-021).
    #[napi]
    pub fn delete_collection(&self, name: String) -> Result<()> {
        self.handle()?.delete_collection(&name).map_err(js_err)
    }

    /// Create an alias that resolves to `target` (CORE-051).
    #[napi]
    pub fn create_alias(&self, alias: String, target: String) -> Result<()> {
        self.handle()?.create_alias(&alias, &target).map_err(js_err)
    }

    /// Delete an alias (CORE-051).
    #[napi]
    pub fn delete_alias(&self, alias: String) -> Result<()> {
        self.handle()?.delete_alias(&alias).map_err(js_err)
    }

    /// Flush acked state to disk (WAL-030b).
    #[napi]
    pub async fn checkpoint(&self) -> Result<()> {
        let db = self.handle()?;
        tokio::task::spawn_blocking(move || db.checkpoint())
            .await
            .map_err(|e| arg_err(e.to_string()))?
            .map_err(js_err)
    }

    /// Write a standalone compacted copy at `path` (STG-070).
    #[napi]
    pub async fn snapshot(&self, path: String) -> Result<()> {
        let db = self.handle()?;
        tokio::task::spawn_blocking(move || db.snapshot(path))
            .await
            .map_err(|e| arg_err(e.to_string()))?
            .map_err(js_err)
    }

    /// Reclaim dead space in place (STG-071).
    #[napi]
    pub async fn vacuum(&self) -> Result<()> {
        let db = self.handle()?;
        tokio::task::spawn_blocking(move || db.vacuum())
            .await
            .map_err(|e| arg_err(e.to_string()))?
            .map_err(js_err)
    }

    /// The on-disk format version this build reads/writes (NFR-11).
    #[napi]
    pub fn format_version(&self) -> u32 {
        1
    }

    /// Flush and close (NODE-013): checkpoint, then drop this handle so the
    /// advisory file lock releases immediately (not on GC). Idempotent — a
    /// second call, and any op after close, is a no-op / `Closed`. Note: a
    /// still-referenced `Collection` keeps the file open, so drop collections
    /// before reopening the same path in-process.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        // Take the handle out under the lock, then drop it OUTSIDE the lock so
        // the (blocking) close-time checkpoint does not hold the mutex.
        let taken = {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| arg_err("database lock poisoned"))?;
            guard.take()
        };
        if let Some(db) = taken {
            // Drop inline rather than on `spawn_blocking`: the handle (and its
            // advisory file lock) must be released before this future resolves so
            // an immediate reopen of the same path succeeds. `spawn_blocking` for
            // a `()`-returning close is not reliably driven to completion on every
            // napi host (e.g. Bun), which would leave the lock held; this future
            // runs on the tokio worker pool, so the brief checkpoint does not
            // block the JS event loop.
            let _ = db.checkpoint(); // best-effort flush
            drop(db); // releases the advisory lock
        }
        Ok(())
    }
}

/// One chunk projected to JS: its trimmed text and byte range in the source.
#[napi(object)]
pub struct JsChunk {
    pub text: String,
    pub start: u32,
    pub end: u32,
}

/// Split `text` into overlapping, UTF-8-safe chunks (SPEC-005 §7). Pure and
/// deterministic; `maxChars`/`overlap` default to 2048/128.
#[napi]
pub fn chunk(text: String, max_chars: Option<u32>, overlap: Option<u32>) -> Vec<JsChunk> {
    let d = ChunkOptions::default();
    let opts = ChunkOptions {
        max_chars: max_chars.map_or(d.max_chars, |v| v as usize),
        overlap: overlap.map_or(d.overlap, |v| v as usize),
    };
    Chunker::new(opts)
        .chunk(&text)
        .into_iter()
        .map(|c| JsChunk {
            text: c.text,
            start: c.byte_range.start as u32,
            end: c.byte_range.end as u32,
        })
        .collect()
}

// ── Collection ──────────────────────────────────────────────────────────────

/// A collection handle (SPEC-004 §4). Cheap to clone (Arc inside).
#[napi]
pub struct Collection {
    inner: veclite::Collection,
}

#[napi]
impl Collection {
    /// Number of live vectors.
    #[napi]
    pub fn len(&self) -> u32 {
        self.inner.len() as u32
    }

    /// Whether the collection holds no live vectors.
    #[napi(js_name = "isEmpty")]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Insert-or-replace one point (API-020). Async: off the event loop. The
    /// optional `sparse` `{indices, values}` sets the hybrid-search lane (SPEC-007).
    #[napi]
    pub async fn upsert(
        &self,
        id: String,
        vector: Float32Array,
        payload: Option<Value>,
        sparse: Option<Value>,
    ) -> Result<()> {
        let v = vector.to_vec(); // owned copy: the JS buffer can't cross threads
        let p = point(id, v, payload, sparse)?;
        let coll = self.inner.clone();
        tokio::task::spawn_blocking(move || coll.upsert(p))
            .await
            .map_err(|e| arg_err(e.to_string()))?
            .map_err(js_err)
    }

    /// Synchronous [`upsert`] — `vector` crosses zero-copy (NODE-012).
    #[napi]
    pub fn upsert_sync(
        &self,
        id: String,
        vector: Float32Array,
        payload: Option<Value>,
        sparse: Option<Value>,
    ) -> Result<()> {
        self.inner
            .upsert(point(id, vector.as_ref().to_vec(), payload, sparse)?)
            .map_err(js_err)
    }

    /// Insert-or-replace a batch (API-020, one WAL entry).
    #[napi]
    pub async fn upsert_batch(&self, points: Vec<JsPoint>) -> Result<()> {
        let owned: Vec<Point> = points
            .into_iter()
            .map(|p| point(p.id, p.vector.to_vec(), p.payload, p.sparse))
            .collect::<Result<_>>()?;
        let coll = self.inner.clone();
        tokio::task::spawn_blocking(move || coll.upsert_batch(owned))
            .await
            .map_err(|e| arg_err(e.to_string()))?
            .map_err(js_err)
    }

    #[napi]
    pub fn upsert_batch_sync(&self, points: Vec<JsPoint>) -> Result<()> {
        let owned: Vec<Point> = points
            .into_iter()
            .map(|p| point(p.id, p.vector.as_ref().to_vec(), p.payload, p.sparse))
            .collect::<Result<_>>()?;
        self.inner.upsert_batch(owned).map_err(js_err)
    }

    /// Force a full recompute of an auto-embed collection's vocabulary (SPEC-005).
    #[napi]
    pub fn refit(&self) -> Result<()> {
        self.inner.refit().map_err(js_err)
    }

    /// Insert-or-replace one text document on an auto-embed collection.
    #[napi]
    pub async fn upsert_text(
        &self,
        id: String,
        text: String,
        payload: Option<Value>,
    ) -> Result<()> {
        let coll = self.inner.clone();
        tokio::task::spawn_blocking(move || coll.upsert_text_batch(vec![(id, text, payload)]))
            .await
            .map_err(|e| arg_err(e.to_string()))?
            .map_err(js_err)
    }

    #[napi]
    pub fn upsert_text_sync(&self, id: String, text: String, payload: Option<Value>) -> Result<()> {
        self.inner
            .upsert_text_batch(vec![(id, text, payload)])
            .map_err(js_err)
    }

    /// k-NN search (API-030). Async: off the event loop.
    #[napi]
    pub async fn search(
        &self,
        query: Float32Array,
        options: Option<SearchOpts>,
    ) -> Result<Vec<JsHit>> {
        let v = query.to_vec();
        let coll = self.inner.clone();
        let hits = tokio::task::spawn_blocking(move || run_search(&coll, &v, options))
            .await
            .map_err(|e| arg_err(e.to_string()))??;
        Ok(hits)
    }

    /// Synchronous [`search`] — `query` crosses zero-copy (NODE-012).
    #[napi]
    pub fn search_sync(
        &self,
        query: Float32Array,
        options: Option<SearchOpts>,
    ) -> Result<Vec<JsHit>> {
        run_search(&self.inner, query.as_ref(), options)
    }

    /// Embed `query` with the collection's provider and search (SPEC-005 §4).
    #[napi]
    pub async fn search_text(
        &self,
        query: String,
        options: Option<SearchOpts>,
    ) -> Result<Vec<JsHit>> {
        let coll = self.inner.clone();
        let hits = tokio::task::spawn_blocking(move || {
            let o = options.unwrap_or_default();
            coll.search_text(&query, o.limit.unwrap_or(10) as usize)
                .map(project)
        })
        .await
        .map_err(|e| arg_err(e.to_string()))?
        .map_err(js_err)?;
        Ok(hits)
    }

    #[napi]
    pub fn search_text_sync(
        &self,
        query: String,
        options: Option<SearchOpts>,
    ) -> Result<Vec<JsHit>> {
        let o = options.unwrap_or_default();
        self.inner
            .search_text(&query, o.limit.unwrap_or(10) as usize)
            .map(project)
            .map_err(js_err)
    }

    /// Hybrid dense+sparse search with RRF fusion (SPEC-007).
    #[napi]
    pub fn hybrid_search(&self, options: HybridOpts) -> Result<Vec<JsHit>> {
        // Own both lanes first so the borrow-based builder can reference them.
        let dense_owned: Option<Vec<f32>> = options.dense.as_ref().map(|d| d.as_ref().to_vec());
        let sparse_owned: Option<SparseVector> = match &options.sparse {
            Some(s) => Some(sparse_from_value(s)?),
            None => None,
        };
        let mut q = self.inner.hybrid_query();
        if let Some(d) = &dense_owned {
            q = q.dense(d);
        }
        if let Some(s) = &sparse_owned {
            q = q.sparse(s);
        }
        if let Some(a) = options.alpha {
            q = q.alpha(a as f32);
        }
        if let Some(k) = options.rrf_k {
            q = q.rrf_k(k as f32);
        }
        if let Some(l) = options.limit {
            q = q.limit(l as usize);
        }
        q.run().map(project).map_err(js_err)
    }

    /// Cursor-based pagination over live points in stable slot order
    /// (API-022). Pass `offsetId` from a prior page's `nextCursor`.
    #[napi]
    pub fn scroll(&self, options: Option<ScrollOpts>) -> Result<JsScrollPage> {
        let o = options.unwrap_or_default();
        let filter = match &o.filter {
            Some(f) => Some(veclite::Filter::from_json(f).map_err(js_err)?),
            None => None,
        };
        let page = self
            .inner
            .scroll(
                o.offset_id.as_deref(),
                o.limit.unwrap_or(100) as usize,
                filter.as_ref(),
            )
            .map_err(js_err)?;
        let points = page
            .points
            .into_iter()
            .map(|p| JsHit {
                id: p.id,
                score: 0.0,
                payload: p.payload,
                vector: Some(Float32Array::new(p.vector)),
            })
            .collect();
        Ok(JsScrollPage {
            points,
            next_cursor: page.next_cursor,
        })
    }

    /// Fetch a point by id (API-021); `null` when absent.
    #[napi]
    pub fn get(&self, id: String) -> Result<Option<JsHit>> {
        let p = self.inner.get(&id).map_err(js_err)?;
        Ok(p.map(|p| JsHit {
            id: p.id,
            score: 0.0,
            payload: p.payload,
            vector: Some(Float32Array::new(p.vector)),
        }))
    }

    /// Delete a point by id (API-022); `true` if it existed.
    #[napi]
    pub fn delete(&self, id: String) -> Result<bool> {
        self.inner.delete(&id).map_err(js_err)
    }

    /// Collection statistics (FR-08/13).
    #[napi]
    pub fn stats(&self) -> JsStats {
        let s = self.inner.stats();
        JsStats {
            name: s.name,
            dimension: s.dimension as u32,
            len: s.len as u32,
            tombstones: s.tombstones as u32,
            auto_embed: s.auto_embed,
        }
    }
}

/// Hybrid query options (SPEC-007).
#[napi(object)]
pub struct HybridOpts {
    pub dense: Option<Float32Array>,
    /// `{ indices: number[], values: number[] }`.
    pub sparse: Option<Value>,
    pub alpha: Option<f64>,
    pub rrf_k: Option<f64>,
    pub limit: Option<u32>,
}

/// `scroll` options (SPEC-004 API-022).
#[napi(object)]
#[derive(Default)]
pub struct ScrollOpts {
    pub limit: Option<u32>,
    pub offset_id: Option<String>,
    pub filter: Option<Value>,
}

/// One page of a `scroll`.
#[napi(object)]
pub struct JsScrollPage {
    pub points: Vec<JsHit>,
    pub next_cursor: Option<String>,
}

/// Collection statistics projected to JS.
#[napi(object)]
pub struct JsStats {
    pub name: String,
    pub dimension: u32,
    pub len: u32,
    pub tombstones: u32,
    pub auto_embed: bool,
}

/// Build a BYO `Point` from JS parts, with an optional `{indices, values}`
/// sparse lane for hybrid search (SPEC-007).
fn point(
    id: String,
    vector: Vec<f32>,
    payload: Option<Value>,
    sparse: Option<Value>,
) -> Result<Point> {
    let mut p = Point::new(id, vector);
    if let Some(v) = payload {
        p = p.payload(v);
    }
    if let Some(s) = sparse {
        p = p.sparse(sparse_from_value(&s)?);
    }
    Ok(p)
}

/// Run a k-NN query with the JS options, projecting to `JsHit`.
fn run_search(
    coll: &veclite::Collection,
    query: &[f32],
    options: Option<SearchOpts>,
) -> Result<Vec<JsHit>> {
    let o = options.unwrap_or_default();
    let mut q = coll.query(query);
    if let Some(l) = o.limit {
        q = q.limit(l as usize);
    }
    if let Some(ef) = o.ef_search {
        q = q.ef_search(ef as usize);
    }
    if let Some(wp) = o.with_payload {
        q = q.with_payload(wp);
    }
    if let Some(wv) = o.with_vector {
        q = q.with_vector(wv);
    }
    if let Some(f) = &o.filter {
        let filter = veclite::Filter::from_json(f).map_err(js_err)?;
        q = q.filter(filter);
    }
    q.run().map(project).map_err(js_err)
}
