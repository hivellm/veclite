//! WebAssembly binding for VecLite (SPEC-012), via wasm-bindgen. Binds the Rust
//! crate directly on its wasm32 profile: the in-memory engine (collection
//! registry, CRUD, brute-force exact search, quantization, scalar-oracle SIMD
//! kernels) plus the portable `.veclite` v1 image codec — no file storage, no
//! mmap, no locks, no threads, no ONNX (WASM-001, gated off wasm32 in the core).
//!
//! The surface here is **synchronous** (execution inside the module is sync);
//! the hand-written JS wrapper (`veclite.js`) adds the camelCase async facade
//! (WASM-020), the simd128/fallback feature-detection loader (WASM-002), and the
//! OPFS backend (WASM-011) on top by calling [`WasmDb::serialize`] /
//! [`WasmDb::from_bytes`].
//!
//! Errors throw a JS `Error` carrying a `code` string property (WASM-021), the
//! FFI constant without the `VL_ERR_` prefix — the same code set as SPEC-010.

use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;

use veclite::chunk::{ChunkOptions, Chunker};
use veclite::{
    CollectionOptions, HnswOptions, Metric, PayloadIndexKind, Point, Quantization, SparseVector,
    VecLite, VecLiteError,
};

// ── errors ───────────────────────────────────────────────────────────────────

/// Map a `VecLiteError` to its JS `code` string (WASM-021 — the SPEC-010 codes,
/// i.e. the FFI constants without the `VL_ERR_` prefix).
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

/// A JS `Error` carrying `code` + the Rust display message (WASM-021).
fn js_err(e: VecLiteError) -> JsValue {
    let err = js_sys::Error::new(&e.to_string());
    let _ = js_sys::Reflect::set(&err, &JsValue::from_str("code"), &code_of(&e).into());
    err.into()
}

/// A JS `Error` for a binding-layer (non-VecLite) failure — `INVALID_ARGUMENT`.
fn arg_err(msg: impl AsRef<str>) -> JsValue {
    let err = js_sys::Error::new(msg.as_ref());
    let _ = js_sys::Reflect::set(&err, &JsValue::from_str("code"), &"INVALID_ARGUMENT".into());
    err.into()
}

type JsResult<T> = Result<T, JsValue>;

/// Serialize any value to a plain JS value (objects, not `Map`s — WASM-021).
fn to_js<T: Serialize>(v: &T) -> JsResult<JsValue> {
    v.serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| arg_err(e.to_string()))
}

/// Read an options `JsValue` (possibly `undefined`/`null`) into a JSON object.
fn opts_value(v: &JsValue) -> JsResult<Value> {
    if v.is_undefined() || v.is_null() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    serde_wasm_bindgen::from_value(v.clone()).map_err(|e| arg_err(format!("invalid options: {e}")))
}

// ── small option readers over a JSON object ──────────────────────────────────

fn get_str<'a>(o: &'a Value, key: &str) -> Option<&'a str> {
    o.get(key).and_then(Value::as_str)
}
fn get_u32(o: &Value, key: &str) -> Option<u32> {
    o.get(key).and_then(Value::as_u64).map(|v| v as u32)
}
fn get_usize(o: &Value, key: &str) -> Option<usize> {
    o.get(key).and_then(Value::as_u64).map(|v| v as usize)
}
fn get_bool(o: &Value, key: &str) -> Option<bool> {
    o.get(key).and_then(Value::as_bool)
}
fn get_f32(o: &Value, key: &str) -> Option<f32> {
    o.get(key).and_then(Value::as_f64).map(|v| v as f32)
}
/// A nested value that is neither absent nor JSON `null`.
fn get_val<'a>(o: &'a Value, key: &str) -> Option<&'a Value> {
    match o.get(key) {
        Some(Value::Null) | None => None,
        Some(v) => Some(v),
    }
}

fn metric_of(s: Option<&str>) -> JsResult<Metric> {
    Ok(match s {
        None | Some("cosine") => Metric::Cosine,
        Some("euclidean") => Metric::Euclidean,
        Some("dotproduct") => Metric::DotProduct,
        Some(other) => return Err(arg_err(format!("unknown metric {other:?}"))),
    })
}

fn payload_index_kind(s: &str) -> JsResult<PayloadIndexKind> {
    Ok(match s {
        "keyword" => PayloadIndexKind::Keyword,
        "integer" => PayloadIndexKind::Integer,
        "float" => PayloadIndexKind::Float,
        other => return Err(arg_err(format!("unknown payload index kind {other:?}"))),
    })
}

fn build_collection_options(o: &Value) -> JsResult<CollectionOptions> {
    let dimension =
        get_usize(o, "dimension").ok_or_else(|| arg_err("createCollection needs a dimension"))?;
    let mut opts = match get_str(o, "autoEmbed") {
        Some(provider) => CollectionOptions::auto_embed(provider, dimension),
        None => CollectionOptions::new(dimension, metric_of(get_str(o, "metric"))?),
    };
    if let Some(bits) = get_u32(o, "quantizationBits") {
        opts = opts.quantization(if bits == 0 {
            Quantization::None
        } else {
            Quantization::Scalar { bits: bits as u8 }
        });
    }
    if let Some(h) = get_val(o, "hnsw") {
        let d = HnswOptions::default();
        opts = opts.hnsw(
            get_usize(h, "m").unwrap_or(d.m),
            get_usize(h, "efConstruction").unwrap_or(d.ef_construction),
            get_usize(h, "efSearch").unwrap_or(d.ef_search),
        );
    }
    if let Some(Value::Array(pairs)) = get_val(o, "payloadIndexes") {
        for pair in pairs {
            let arr = pair
                .as_array()
                .ok_or_else(|| arg_err("payloadIndexes entries must be [key, kind] pairs"))?;
            let key = arr
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| arg_err("payload index needs a key"))?;
            let kind = arr
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| arg_err("payload index needs a kind"))?;
            opts = opts.payload_index(key, payload_index_kind(kind)?);
        }
    }
    Ok(opts)
}

fn sparse_from_value(v: &Value) -> JsResult<SparseVector> {
    serde_json::from_value(v.clone()).map_err(|e| arg_err(format!("invalid sparse vector: {e}")))
}

/// Build a BYO `Point` from JS parts, with an optional `{indices, values}`
/// sparse lane for hybrid search (SPEC-007).
fn build_point(
    id: String,
    vector: Vec<f32>,
    payload: Option<Value>,
    sparse: Option<Value>,
) -> JsResult<Point> {
    let mut p = Point::new(id, vector);
    if let Some(v) = payload {
        p = p.payload(v);
    }
    if let Some(s) = sparse {
        p = p.sparse(sparse_from_value(&s)?);
    }
    Ok(p)
}

// ── projected shapes (serialized to plain JS objects) ────────────────────────

#[derive(Serialize)]
struct HitOut {
    id: String,
    score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vector: Option<Vec<f32>>,
}

impl HitOut {
    fn ranked(h: veclite::Hit) -> Self {
        HitOut {
            id: h.id,
            score: f64::from(h.score),
            payload: h.payload,
            vector: h.vector,
        }
    }
    /// A point projection (get/scroll): score 0, vector always present.
    fn point(p: veclite::Point) -> Self {
        HitOut {
            id: p.id,
            score: 0.0,
            payload: p.payload,
            vector: Some(p.vector),
        }
    }
}

#[derive(Serialize)]
struct ScrollOut {
    points: Vec<HitOut>,
    #[serde(rename = "nextCursor")]
    next_cursor: Option<String>,
}

#[derive(Serialize)]
struct StatsOut {
    name: String,
    dimension: u32,
    len: u32,
    tombstones: u32,
    #[serde(rename = "autoEmbed")]
    auto_embed: bool,
}

#[derive(Serialize)]
struct ChunkOut {
    text: String,
    start: u32,
    end: u32,
}

fn project(hits: Vec<veclite::Hit>) -> Vec<HitOut> {
    hits.into_iter().map(HitOut::ranked).collect()
}

// ── Database ─────────────────────────────────────────────────────────────────

/// A VecLite database handle (SPEC-004 §1). On wasm it is always in-memory; a
/// persistent variant is materialized by the JS wrapper via serialize/OPFS
/// (WASM-011). `VecLite` is internally `Arc`, so cloning is cheap.
#[wasm_bindgen]
pub struct WasmDb {
    inner: VecLite,
}

#[wasm_bindgen]
impl WasmDb {
    /// Open an ephemeral in-memory database (FR-02).
    #[wasm_bindgen]
    pub fn memory() -> WasmDb {
        WasmDb {
            inner: VecLite::memory(),
        }
    }

    /// Load a database from a `.veclite` v1 file image (WASM-010): bytes written
    /// by [`WasmDb::serialize`] or by native VecLite. The JS wrapper's OPFS
    /// backend and `deserialize()` entry point both route through here.
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(bytes: &[u8]) -> JsResult<WasmDb> {
        Ok(WasmDb {
            inner: VecLite::deserialize(bytes).map_err(js_err)?,
        })
    }

    /// Serialize the whole database to a `.veclite` v1 file image (WASM-010) as a
    /// `Uint8Array` — a compacted, single-generation image that native VecLite
    /// opens and [`WasmDb::from_bytes`] loads. The OPFS backend writes these bytes.
    #[wasm_bindgen]
    pub fn serialize(&self) -> JsResult<Vec<u8>> {
        self.inner.serialize().map_err(js_err)
    }

    /// Create a collection (CORE-020). `options` is a plain JS object mirroring
    /// SPEC-010's `CollectionOptions` (camelCase).
    #[wasm_bindgen(js_name = createCollection)]
    pub fn create_collection(&self, name: &str, options: JsValue) -> JsResult<WasmCollection> {
        let o = opts_value(&options)?;
        let opts = build_collection_options(&o)?;
        let inner = self.inner.create_collection(name, opts).map_err(js_err)?;
        Ok(WasmCollection { inner })
    }

    /// Get a collection handle by name or alias (CORE-051).
    #[wasm_bindgen]
    pub fn collection(&self, name: &str) -> JsResult<WasmCollection> {
        Ok(WasmCollection {
            inner: self.inner.collection(name).map_err(js_err)?,
        })
    }

    /// List collection names.
    #[wasm_bindgen(js_name = listCollections)]
    pub fn list_collections(&self) -> Vec<String> {
        self.inner.list_collections()
    }

    /// Delete a collection (CORE-021).
    #[wasm_bindgen(js_name = deleteCollection)]
    pub fn delete_collection(&self, name: &str) -> JsResult<()> {
        self.inner.delete_collection(name).map_err(js_err)
    }

    /// Create an alias that resolves to `target` (CORE-051).
    #[wasm_bindgen(js_name = createAlias)]
    pub fn create_alias(&self, alias: &str, target: &str) -> JsResult<()> {
        self.inner.create_alias(alias, target).map_err(js_err)
    }

    /// Delete an alias (CORE-051).
    #[wasm_bindgen(js_name = deleteAlias)]
    pub fn delete_alias(&self, alias: &str) -> JsResult<()> {
        self.inner.delete_alias(alias).map_err(js_err)
    }

    /// The on-disk format version this build reads/writes (NFR-11).
    #[wasm_bindgen(js_name = formatVersion)]
    pub fn format_version(&self) -> u32 {
        1
    }

    /// Compaction is a no-op on wasm (WASM-020): the in-memory image is already
    /// compacted, and `serialize()` writes the compacted form. Present so the
    /// SPEC-010 surface is complete.
    #[wasm_bindgen]
    pub fn vacuum(&self) {}
}

// ── Collection ───────────────────────────────────────────────────────────────

/// A collection handle (SPEC-004 §4). Cheap to clone (Arc inside).
#[wasm_bindgen]
pub struct WasmCollection {
    inner: veclite::Collection,
}

#[wasm_bindgen]
impl WasmCollection {
    /// Number of live vectors.
    #[wasm_bindgen]
    pub fn len(&self) -> u32 {
        self.inner.len() as u32
    }

    /// Whether the collection holds no live vectors.
    #[wasm_bindgen(js_name = isEmpty)]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Insert-or-replace one point (API-020). `vector` is a `Float32Array`; the
    /// optional `sparse` `{indices, values}` sets the hybrid lane (SPEC-007).
    #[wasm_bindgen]
    pub fn upsert(
        &self,
        id: String,
        vector: Vec<f32>,
        payload: JsValue,
        sparse: JsValue,
    ) -> JsResult<()> {
        let p = build_point(id, vector, opt_json(&payload)?, opt_json(&sparse)?)?;
        self.inner.upsert(p).map_err(js_err)
    }

    /// Insert-or-replace a batch (API-020): an array of
    /// `{ id, vector, payload?, sparse? }`.
    #[wasm_bindgen(js_name = upsertBatch)]
    pub fn upsert_batch(&self, points: JsValue) -> JsResult<()> {
        let arr: Value = serde_wasm_bindgen::from_value(points)
            .map_err(|e| arg_err(format!("invalid points: {e}")))?;
        let items = arr
            .as_array()
            .ok_or_else(|| arg_err("upsertBatch expects an array"))?;
        let mut owned = Vec::with_capacity(items.len());
        for it in items {
            let id = get_str(it, "id")
                .ok_or_else(|| arg_err("each point needs an id"))?
                .to_owned();
            let vector: Vec<f32> = it
                .get("vector")
                .and_then(Value::as_array)
                .ok_or_else(|| arg_err("each point needs a vector"))?
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            owned.push(build_point(
                id,
                vector,
                get_val(it, "payload").cloned(),
                get_val(it, "sparse").cloned(),
            )?);
        }
        self.inner.upsert_batch(owned).map_err(js_err)
    }

    /// Insert-or-replace one text document on an auto-embed collection.
    #[wasm_bindgen(js_name = upsertText)]
    pub fn upsert_text(&self, id: String, text: String, payload: JsValue) -> JsResult<()> {
        self.inner
            .upsert_text_batch(vec![(id, text, opt_json(&payload)?)])
            .map_err(js_err)
    }

    /// Force a full recompute of an auto-embed collection's vocabulary (SPEC-005).
    #[wasm_bindgen]
    pub fn refit(&self) -> JsResult<()> {
        self.inner.refit().map_err(js_err)
    }

    /// k-NN search (API-030). On wasm this is an exact brute-force scan (no HNSW,
    /// ADR-0002). Returns an array of `{ id, score, payload?, vector? }`.
    #[wasm_bindgen]
    pub fn search(&self, query: Vec<f32>, options: JsValue) -> JsResult<JsValue> {
        let o = opts_value(&options)?;
        let mut q = self.inner.query(&query);
        if let Some(l) = get_usize(&o, "limit") {
            q = q.limit(l);
        }
        if let Some(ef) = get_usize(&o, "efSearch") {
            q = q.ef_search(ef);
        }
        if let Some(wp) = get_bool(&o, "withPayload") {
            q = q.with_payload(wp);
        }
        if let Some(wv) = get_bool(&o, "withVector") {
            q = q.with_vector(wv);
        }
        if let Some(f) = get_val(&o, "filter") {
            q = q.filter(veclite::Filter::from_json(f).map_err(js_err)?);
        }
        let hits = q.run().map_err(js_err)?;
        to_js(&project(hits))
    }

    /// Embed `query` with the collection's provider and search (SPEC-005 §4).
    #[wasm_bindgen(js_name = searchText)]
    pub fn search_text(&self, query: &str, options: JsValue) -> JsResult<JsValue> {
        let o = opts_value(&options)?;
        let hits = self
            .inner
            .search_text(query, get_usize(&o, "limit").unwrap_or(10))
            .map_err(js_err)?;
        to_js(&project(hits))
    }

    /// Hybrid dense+sparse search with RRF fusion (SPEC-007).
    #[wasm_bindgen(js_name = hybridSearch)]
    pub fn hybrid_search(&self, options: JsValue) -> JsResult<JsValue> {
        let o = opts_value(&options)?;
        let dense: Option<Vec<f32>> = get_val(&o, "dense").and_then(Value::as_array).map(|a| {
            a.iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect()
        });
        let sparse: Option<SparseVector> = match get_val(&o, "sparse") {
            Some(s) => Some(sparse_from_value(s)?),
            None => None,
        };
        let mut q = self.inner.hybrid_query();
        if let Some(d) = &dense {
            q = q.dense(d);
        }
        if let Some(s) = &sparse {
            q = q.sparse(s);
        }
        if let Some(a) = get_f32(&o, "alpha") {
            q = q.alpha(a);
        }
        if let Some(k) = get_f32(&o, "rrfK") {
            q = q.rrf_k(k);
        }
        if let Some(l) = get_usize(&o, "limit") {
            q = q.limit(l);
        }
        let hits = q.run().map_err(js_err)?;
        to_js(&project(hits))
    }

    /// Cursor-based pagination over live points in stable slot order (API-022).
    #[wasm_bindgen]
    pub fn scroll(&self, options: JsValue) -> JsResult<JsValue> {
        let o = opts_value(&options)?;
        let filter = match get_val(&o, "filter") {
            Some(f) => Some(veclite::Filter::from_json(f).map_err(js_err)?),
            None => None,
        };
        let page = self
            .inner
            .scroll(
                get_str(&o, "offsetId"),
                get_usize(&o, "limit").unwrap_or(100),
                filter.as_ref(),
            )
            .map_err(js_err)?;
        let out = ScrollOut {
            points: page.points.into_iter().map(HitOut::point).collect(),
            next_cursor: page.next_cursor,
        };
        to_js(&out)
    }

    /// Fetch a point by id (API-021); `null` when absent.
    #[wasm_bindgen]
    pub fn get(&self, id: &str) -> JsResult<JsValue> {
        match self.inner.get(id).map_err(js_err)? {
            Some(p) => to_js(&HitOut::point(p)),
            None => Ok(JsValue::NULL),
        }
    }

    /// Delete a point by id (API-022); `true` if it existed.
    #[wasm_bindgen]
    pub fn delete(&self, id: &str) -> JsResult<bool> {
        self.inner.delete(id).map_err(js_err)
    }

    /// Collection statistics (FR-08/13).
    #[wasm_bindgen]
    pub fn stats(&self) -> JsResult<JsValue> {
        let s = self.inner.stats();
        to_js(&StatsOut {
            name: s.name,
            dimension: s.dimension as u32,
            len: s.len as u32,
            tombstones: s.tombstones as u32,
            auto_embed: s.auto_embed,
        })
    }
}

/// Read an optional JS argument (`undefined`/`null` → `None`) into JSON.
fn opt_json(v: &JsValue) -> JsResult<Option<Value>> {
    if v.is_undefined() || v.is_null() {
        return Ok(None);
    }
    serde_wasm_bindgen::from_value(v.clone())
        .map(Some)
        .map_err(|e| arg_err(format!("invalid argument: {e}")))
}

/// Split `text` into overlapping, UTF-8-safe chunks (SPEC-005 §7). Pure and
/// deterministic; `maxChars`/`overlap` default to 2048/128.
#[wasm_bindgen]
pub fn chunk(text: &str, max_chars: Option<u32>, overlap: Option<u32>) -> JsResult<JsValue> {
    let d = ChunkOptions::default();
    let opts = ChunkOptions {
        max_chars: max_chars.map_or(d.max_chars, |v| v as usize),
        overlap: overlap.map_or(d.overlap, |v| v as usize),
    };
    let out: Vec<ChunkOut> = Chunker::new(opts)
        .chunk(text)
        .into_iter()
        .map(|c| ChunkOut {
            text: c.text,
            start: c.byte_range.start as u32,
            end: c.byte_range.end as u32,
        })
        .collect();
    to_js(&out)
}
