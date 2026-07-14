//! Collection handle and in-memory slot storage (SPEC-001 §3–4).
//!
//! Storage is append-mostly, mirroring the on-disk design (SPEC-002 STG-002):
//! vectors live in a flat slot-major block; updates and deletes tombstone the
//! old slot and appends take a fresh one. Space reclamation is `vacuum`'s job
//! (phase2d); the HNSW index (phase1b) shares the same slot numbering.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::{Mutex, RwLock};

use crate::embedding::Embedder;
use crate::error::{Result, VecLiteError};
use crate::filter::Filter;
#[cfg(not(target_arch = "wasm32"))]
use crate::filter::index::PayloadIndexes;
#[cfg(not(target_arch = "wasm32"))]
use crate::index::HnswIndex;
use crate::options::{CollectionOptions, Metric, Quantization};
#[cfg(not(target_arch = "wasm32"))]
use crate::persist::wal_body;
use crate::point::{Hit, Point, SparseVector, validate_id};
use crate::quantization::traits::{QuantizationMethod, QuantizationParams};
use crate::quantization::{BinaryQuantization, ScalarQuantization};
use crate::query::QueryBuilder;
use crate::simd::{cosine_similarity, dot_product, euclidean_distance};
#[cfg(not(target_arch = "wasm32"))]
use crate::storage::wal::WalOp;

/// Sink for write-ahead-log records, implemented by the persistence layer. The
/// trait keeps the collection decoupled from the (native-only) `Persistence`
/// type: a memory collection holds `None`, a file-backed one holds the shared
/// journal. `op` is the WAL op byte (`WalOp::to_byte`). On wasm32 there is no
/// persistence, so it is never implemented — dead there by design.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) trait WalSink: Send + Sync {
    /// Append one mutation to the WAL before it is applied to memory.
    fn log(&self, coll_id: u32, op: u8, body: Vec<u8>) -> Result<()>;
    /// Called after a write is applied: drives a checkpoint if the WAL crossed
    /// its size threshold (WAL-030a).
    fn after_write(&self) -> Result<()>;
}

/// Shared state behind every [`Collection`] handle for one collection.
pub(crate) struct CollectionInner {
    /// Current name; renames update it in place (CORE-022).
    pub(crate) name: RwLock<String>,
    /// Immutable configuration (CORE-016).
    pub(crate) config: CollectionOptions,
    /// Registry id, stamped in WAL entries and CONFIG segments. 0 for memory
    /// collections (no persistence).
    pub(crate) coll_id: u32,
    /// Set by `delete_collection`; stale handles then fail with
    /// `CollectionNotFound` (CORE-021).
    pub(crate) deleted: AtomicBool,
    /// The database's shared WAL, or `None` for a memory collection (writes are
    /// not logged). Never set on wasm32 (no file storage) — dead there.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    persistence: Option<Arc<dyn WalSink>>,
    /// Text embedder for an auto-embed collection (SPEC-005), or `None` for a
    /// BYO-vectors collection. Its vocabulary is a function of the live `_text`
    /// corpus, rebuilt lazily when `text_dirty` is set.
    embedder: Option<Mutex<Box<dyn Embedder>>>,
    /// Set when `_text` changed and the embedder vocabulary/document vectors are
    /// stale; the next search (or `refit`) recomputes them (SPEC-005 §5).
    text_dirty: AtomicBool,
    data: RwLock<CollectionData>,
}

/// Slot-major in-memory storage.
struct CollectionData {
    /// Flat vector block: slot `s` occupies `s*dim .. (s+1)*dim`.
    vectors: Vec<f32>,
    /// Slot → id, including tombstoned slots (needed by the index layer).
    ids: Vec<String>,
    /// Live ids only.
    id_to_slot: HashMap<String, usize>,
    /// Dead slots awaiting vacuum (roaring bitmap once storage lands).
    tombstones: HashSet<usize>,
    /// Slot → payload (cleared on tombstone).
    payloads: Vec<Option<serde_json::Value>>,
    /// Slot → sparse lane (cleared on tombstone).
    sparses: Vec<Option<SparseVector>>,
    /// Native HNSW graph keyed by slot number (phase1b); `None` for
    /// `DotProduct` collections since hnsw_rs's `DistDot` panics on
    /// unnormalized vectors (ADR-0002). Node ids never change identity —
    /// only `reindex` rebuilds this wholesale to purge tombstoned slots.
    #[cfg(not(target_arch = "wasm32"))]
    index: Option<HnswIndex>,
    /// Flat SQ-8/binary code block from the last `reindex`; empty until the
    /// first reindex or under `Quantization::None` (SPEC-001 §6). Always a
    /// batch artifact — never kept in sync per-upsert (CORE-041 parity).
    codes: Vec<u8>,
    /// Parameters to decode `codes`; `None` exactly when `codes` is empty.
    quant_params: Option<QuantizationParams>,
    /// Declared payload indexes (SPEC-006 §3), maintained per write and used to
    /// pre-filter selective queries. Empty when no index is declared. Native
    /// only (roaring); wasm32 filters by scan.
    #[cfg(not(target_arch = "wasm32"))]
    payload_indexes: PayloadIndexes,
}

impl CollectionData {
    #[cfg(not(target_arch = "wasm32"))]
    fn empty(index: Option<HnswIndex>, payload_indexes: PayloadIndexes) -> Self {
        CollectionData {
            vectors: Vec::new(),
            ids: Vec::new(),
            id_to_slot: HashMap::new(),
            tombstones: HashSet::new(),
            payloads: Vec::new(),
            sparses: Vec::new(),
            index,
            codes: Vec::new(),
            quant_params: None,
            payload_indexes,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn empty() -> Self {
        CollectionData {
            vectors: Vec::new(),
            ids: Vec::new(),
            id_to_slot: HashMap::new(),
            tombstones: HashSet::new(),
            payloads: Vec::new(),
            sparses: Vec::new(),
            codes: Vec::new(),
            quant_params: None,
        }
    }
}

/// Allocation hint handed to a freshly created HNSW graph. Not a hard cap —
/// hnsw_rs reallocates past it (SPEC-001 CORE-030). Referenced on all targets
/// as the default capacity arg; ignored on wasm32 (no index).
const INITIAL_INDEX_CAPACITY: usize = 1024;

/// Per-query candidate-list bounds (SPEC-001 CORE-031). Enforced in
/// `execute_query` so the brute-force/wasm path rejects out-of-range
/// `ef_search` the same way the index path does.
const EF_SEARCH_BOUNDS: std::ops::RangeInclusive<usize> = 1..=4096;

/// Maximum payload size, checked on the serialized form (SPEC-002 §8, FLT-001).
const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

impl CollectionInner {
    /// Build a new collection's shared state, including its HNSW index
    /// (native only). Propagates `HnswIndex::new`'s `m`/`ef_construction`
    /// bounds check (SPEC-001 CORE-031); `DotProduct` collections get no
    /// index (ADR-0002 — `DistDot` panics on unnormalized vectors).
    pub(crate) fn new(
        name: String,
        config: CollectionOptions,
        coll_id: u32,
        persistence: Option<Arc<dyn WalSink>>,
    ) -> Result<Self> {
        Self::with_capacity(name, config, coll_id, persistence, INITIAL_INDEX_CAPACITY)
    }

    /// As [`new`](Self::new) but with an explicit HNSW capacity hint — used when
    /// loading a collection whose live count is already known (avoids an early
    /// graph reallocation).
    pub(crate) fn with_capacity(
        name: String,
        config: CollectionOptions,
        coll_id: u32,
        persistence: Option<Arc<dyn WalSink>>,
        capacity: usize,
    ) -> Result<Self> {
        #[cfg(not(target_arch = "wasm32"))]
        let data = {
            let index = match config.metric {
                Metric::DotProduct => None,
                _ => Some(HnswIndex::new(
                    config.metric,
                    config.dimension,
                    config.hnsw.m,
                    config.hnsw.ef_construction,
                    capacity.max(INITIAL_INDEX_CAPACITY),
                )?),
            };
            CollectionData::empty(index, PayloadIndexes::new(&config.payload_indexes))
        };
        #[cfg(target_arch = "wasm32")]
        let data = {
            let _ = capacity;
            CollectionData::empty()
        };

        // Build the text embedder for an auto-embed collection; an unknown
        // provider fails fast here with `UnsupportedProvider` (EMB-021).
        let embedder = match &config.embedding_provider {
            Some(name) => Some(Mutex::new(crate::embedding::build_provider(
                name,
                config.dimension,
            )?)),
            None => None,
        };

        // Auto-embed collections start "dirty" so the first search rebuilds the
        // vocabulary from the loaded/ingested `_text` (mirrors the HNSW rebuild).
        let text_dirty = AtomicBool::new(embedder.is_some());

        Ok(CollectionInner {
            name: RwLock::new(name),
            deleted: AtomicBool::new(false),
            coll_id,
            persistence,
            embedder,
            text_dirty,
            data: RwLock::new(data),
            config,
        })
    }
}

/// Embed `text`, substituting a deterministic non-zero placeholder for an
/// all-zero result (empty vocabulary or fully out-of-vocabulary text). The
/// placeholder keeps cosine collections valid until `refit` replaces it with the
/// real embedding (SPEC-005 §5).
fn embed_nonzero(embedder: &dyn Embedder, text: &str, dim: usize) -> Result<Vec<f32>> {
    let v = embedder.embed(text)?;
    if v.iter().any(|&x| x != 0.0) {
        return Ok(v);
    }
    let mut placeholder = vec![0.0f32; dim];
    if let Some(first) = placeholder.first_mut() {
        *first = 1.0;
    }
    Ok(placeholder)
}

/// Handle to a collection. Cheap to clone; `Send + Sync` (CORE-050).
#[derive(Clone)]
pub struct Collection {
    pub(crate) inner: Arc<CollectionInner>,
}

/// A point validated and normalized, ready to apply under the write lock.
struct PreparedPoint {
    id: String,
    vector: Vec<f32>,
    sparse: Option<SparseVector>,
    payload: Option<serde_json::Value>,
}

impl Collection {
    /// Fail with `CollectionNotFound` once the collection was deleted
    /// (CORE-021).
    fn guard(&self) -> Result<()> {
        if self.inner.deleted.load(Ordering::Acquire) {
            return Err(VecLiteError::CollectionNotFound(
                self.inner.name.read().clone(),
            ));
        }
        Ok(())
    }

    /// Validate ingest rules (CORE-010, CORE-012..014) and normalize for Cosine.
    /// Runs outside any lock; on error nothing was modified. `allow_reserved` is
    /// set only for the internal text path and WAL replay, which legitimately
    /// carry the system `_text` key (SPEC-005 EMB-022); the public path rejects
    /// reserved `_`-prefixed payload keys.
    fn prepare_inner(&self, point: Point, allow_reserved: bool) -> Result<PreparedPoint> {
        validate_id(&point.id)?;
        let expected = self.inner.config.dimension;
        if point.vector.len() != expected {
            return Err(VecLiteError::DimensionMismatch {
                expected,
                got: point.vector.len(),
            });
        }
        if point.vector.iter().any(|v| !v.is_finite()) {
            return Err(VecLiteError::InvalidArgument(format!(
                "vector for id {:?} contains NaN or infinite values",
                point.id
            )));
        }
        if let Some(payload) = point.payload.as_ref() {
            // Top-level payload keys beginning with `_` are reserved (SPEC-006
            // FLT-002, e.g. `_text`); reject rather than silently store them.
            if !allow_reserved {
                if let Some(obj) = payload.as_object() {
                    if let Some(reserved) = obj.keys().find(|k| k.starts_with('_')) {
                        return Err(VecLiteError::InvalidArgument(format!(
                            "payload key {reserved:?} is reserved (keys starting with '_')"
                        )));
                    }
                }
            }
            // Payload size limit (SPEC-002 §8 / FLT-001): 16 MiB. Checked on the
            // serialized (uncompressed) form — a conservative bound, since the
            // stored form is compressed.
            let size = serde_json::to_vec(payload)
                .map(|b| b.len())
                .unwrap_or(usize::MAX);
            if size > MAX_PAYLOAD_BYTES {
                return Err(VecLiteError::InvalidArgument(format!(
                    "payload for id {:?} is {size} bytes, over the {MAX_PAYLOAD_BYTES}-byte limit",
                    point.id
                )));
            }
        }
        if let Some(sparse) = &point.sparse {
            sparse.validate()?;
            // Auto-embed collections own the sparse lane; a BYO sparse vector on
            // one is a mode conflict (SPEC-007 HYB-002). Skipped on replay.
            if !allow_reserved && self.inner.embedder.is_some() {
                return Err(VecLiteError::InvalidArgument(
                    "auto-embed collections manage the sparse lane; do not supply an explicit sparse vector"
                        .to_owned(),
                ));
            }
        }
        let mut vector = point.vector;
        if self.inner.config.metric == Metric::Cosine {
            let norm = vector
                .iter()
                .map(|v| f64::from(*v) * f64::from(*v))
                .sum::<f64>()
                .sqrt();
            if norm == 0.0 {
                return Err(VecLiteError::InvalidArgument(format!(
                    "zero vector for id {:?} is not allowed with the cosine metric",
                    point.id
                )));
            }
            #[allow(clippy::cast_possible_truncation)]
            for v in &mut vector {
                *v = (f64::from(*v) / norm) as f32;
            }
        }
        Ok(PreparedPoint {
            id: point.id,
            vector,
            sparse: point.sparse,
            payload: point.payload,
        })
    }

    /// Insert-or-replace one point (SPEC-004 API-020).
    pub fn upsert(&self, point: Point) -> Result<()> {
        self.upsert_batch(vec![point])
    }

    /// Insert-or-replace a batch. The batch is the atomic unit: every point is
    /// validated before any is applied, all become visible together, and the
    /// whole batch is one WAL entry (SPEC-003 WAL-012). Order: validate → log →
    /// apply, so a rejected point never reaches the WAL.
    pub fn upsert_batch(&self, points: Vec<Point>) -> Result<()> {
        self.upsert_batch_inner(points, false)
    }

    /// Shared upsert path. `allow_reserved` permits the system `_text` key for
    /// the text API and WAL replay (SPEC-005 EMB-022).
    fn upsert_batch_inner(&self, points: Vec<Point>, allow_reserved: bool) -> Result<()> {
        self.guard()?;
        // Encode the WAL body from the originals before validation consumes
        // them (only when persistent; discarded if validation then fails).
        #[cfg(not(target_arch = "wasm32"))]
        let body = self.encode_wal(&points)?;
        let prepared: Vec<PreparedPoint> = points
            .into_iter()
            .map(|p| self.prepare_inner(p, allow_reserved))
            .collect::<Result<_>>()?;
        #[cfg(not(target_arch = "wasm32"))]
        self.log(WalOp::UpsertBatch, body)?;
        self.apply_prepared(prepared);
        #[cfg(not(target_arch = "wasm32"))]
        self.after_write()?;
        Ok(())
    }

    /// Insert-or-replace one text document (SPEC-005 §4). The text is embedded
    /// with the collection's provider and stored under `_text` for later
    /// `refit`/reopen. `InvalidArgument` on a BYO (non-auto-embed) collection
    /// (EMB-021).
    pub fn upsert_text(&self, id: impl Into<String>, text: impl Into<String>) -> Result<()> {
        self.upsert_text_batch(vec![(id.into(), text.into(), None)])
    }

    /// Insert-or-replace one text document with a user payload (which MUST NOT
    /// use reserved `_`-prefixed keys).
    pub fn upsert_text_with(
        &self,
        id: impl Into<String>,
        text: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<()> {
        self.upsert_text_batch(vec![(id.into(), text.into(), Some(payload))])
    }

    /// Insert-or-replace a batch of `(id, text, payload?)` documents (SPEC-005
    /// §4). Each is embedded now with the current vocabulary; the vocabulary is
    /// then marked stale so the next search recomputes it exactly from the full
    /// `_text` corpus (SPEC-005 §5).
    pub fn upsert_text_batch(
        &self,
        items: Vec<(String, String, Option<serde_json::Value>)>,
    ) -> Result<()> {
        self.guard()?;
        let embedder = self.inner.embedder.as_ref().ok_or_else(|| {
            VecLiteError::InvalidArgument(
                "text operations require an auto-embed collection (use CollectionOptions::auto_embed)"
                    .to_owned(),
            )
        })?;

        let mut points = Vec::with_capacity(items.len());
        {
            let emb = embedder.lock();
            for (id, text, user_payload) in items {
                // The user payload may not carry reserved keys.
                if let Some(obj) = user_payload.as_ref().and_then(|v| v.as_object()) {
                    if let Some(reserved) = obj.keys().find(|k| k.starts_with('_')) {
                        return Err(VecLiteError::InvalidArgument(format!(
                            "payload key {reserved:?} is reserved (keys starting with '_')"
                        )));
                    }
                }
                let vector = embed_nonzero(&**emb, &text, self.inner.config.dimension)?;
                let mut payload = user_payload.unwrap_or_else(|| serde_json::json!({}));
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("_text".to_owned(), serde_json::Value::String(text));
                }
                points.push(Point {
                    id,
                    vector,
                    sparse: None,
                    payload: Some(payload),
                });
            }
        }
        self.upsert_batch_inner(points, true)?;
        self.inner.text_dirty.store(true, Ordering::Release);
        Ok(())
    }

    /// Embed `query` with the collection's provider and search (SPEC-005 §4).
    /// Recomputes a stale vocabulary first so results reflect the full corpus.
    /// `InvalidArgument` on a BYO collection.
    pub fn search_text(&self, query: &str, limit: usize) -> Result<Vec<Hit>> {
        self.guard()?;
        if self.inner.embedder.is_none() {
            return Err(VecLiteError::InvalidArgument(
                "text search requires an auto-embed collection".to_owned(),
            ));
        }
        self.refit_if_dirty()?;
        let embedder = self
            .inner
            .embedder
            .as_ref()
            .ok_or_else(|| VecLiteError::InvalidArgument("no embedder".to_owned()))?;
        let vector = embedder.lock().embed(query)?;
        self.search(&vector, limit)
    }

    /// Recompute the vocabulary from all live `_text` and re-embed every text
    /// document (SPEC-005 EMB-031/032). Exact, explicit, potentially slow.
    /// `InvalidArgument` on a BYO collection.
    pub fn refit(&self) -> Result<()> {
        self.guard()?;
        if self.inner.embedder.is_none() {
            return Err(VecLiteError::InvalidArgument(
                "refit requires an auto-embed collection".to_owned(),
            ));
        }
        self.do_refit()
    }

    /// Run [`do_refit`](Self::do_refit) only when `_text` changed since the last
    /// recompute (lazy, amortizes a batch of text upserts into one rebuild).
    fn refit_if_dirty(&self) -> Result<()> {
        if self.inner.text_dirty.swap(false, Ordering::AcqRel) {
            self.do_refit()?;
        }
        Ok(())
    }

    /// Gather live `_text`, fit the embedder on the full corpus, and re-embed
    /// every text document so all stored vectors share one vocabulary.
    fn do_refit(&self) -> Result<()> {
        let Some(embedder) = self.inner.embedder.as_ref() else {
            return Ok(());
        };
        // Snapshot live text documents in slot order, keeping the FULL payload
        // (user keys + `_text`) so re-embedding never drops metadata.
        let live: Vec<(String, String, serde_json::Value)> = {
            let data = self.inner.data.read();
            let mut out = Vec::new();
            for slot in 0..data.ids.len() {
                if data.id_to_slot.get(&data.ids[slot]) == Some(&slot) {
                    if let Some(payload) = data.payloads[slot].as_ref() {
                        if let Some(text) = payload.get("_text").and_then(|t| t.as_str()) {
                            out.push((data.ids[slot].clone(), text.to_owned(), payload.clone()));
                        }
                    }
                }
            }
            out
        };
        if live.is_empty() {
            return Ok(());
        }
        // Fit on the full corpus, then re-embed each document with the new vocab.
        let repoints: Vec<Point> = {
            let mut emb = embedder.lock();
            let corpus: Vec<&str> = live.iter().map(|(_, t, _)| t.as_str()).collect();
            emb.fit(&corpus)?;
            let mut out = Vec::with_capacity(live.len());
            for (id, text, payload) in &live {
                out.push(Point {
                    id: id.clone(),
                    vector: embed_nonzero(&**emb, text, self.inner.config.dimension)?,
                    sparse: None,
                    payload: Some(payload.clone()),
                });
            }
            out
        };
        self.upsert_batch_inner(repoints, true)?;
        self.inner.text_dirty.store(false, Ordering::Release);
        Ok(())
    }

    /// Apply validated points under the write lock (shared by the public path
    /// and WAL replay — replay skips the log/after-write around this).
    fn apply_prepared(&self, prepared: Vec<PreparedPoint>) {
        let mut data = self.inner.data.write();
        for p in prepared {
            apply_upsert(&mut data, p, self.inner.config.dimension);
        }
    }

    /// Fetch a point by id; `None` when absent (API-021). Cosine collections
    /// return the stored (normalized) vector (CORE-014).
    pub fn get(&self, id: &str) -> Result<Option<Point>> {
        self.guard()?;
        let data = self.inner.data.read();
        let Some(&slot) = data.id_to_slot.get(id) else {
            return Ok(None);
        };
        let dim = self.inner.config.dimension;
        Ok(Some(Point {
            id: data.ids[slot].clone(),
            vector: data.vectors[slot * dim..(slot + 1) * dim].to_vec(),
            sparse: data.sparses[slot].clone(),
            payload: data.payloads[slot].clone(),
        }))
    }

    /// Delete by id; `false` when the id was absent (API-021).
    pub fn delete(&self, id: &str) -> Result<bool> {
        Ok(self.delete_batch(&[id])? == 1)
    }

    /// Delete a batch of ids; returns how many existed. One atomic WAL entry,
    /// like `upsert_batch`.
    pub fn delete_batch(&self, ids: &[&str]) -> Result<usize> {
        self.guard()?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let body = self
                .inner
                .persistence
                .as_ref()
                .map(|_| {
                    let owned: Vec<String> = ids.iter().map(|s| (*s).to_owned()).collect();
                    wal_body::encode(&owned)
                })
                .transpose()?;
            self.log(WalOp::DeleteBatch, body)?;
        }
        let removed = {
            let mut data = self.inner.data.write();
            ids.iter().filter(|id| tombstone(&mut data, id)).count()
        };
        #[cfg(not(target_arch = "wasm32"))]
        self.after_write()?;
        Ok(removed)
    }

    /// Number of live vectors. Returns 0 after the collection was deleted
    /// (the fallible operations report `CollectionNotFound` instead —
    /// CORE-021; `len` stays infallible per SPEC-004 §4).
    pub fn len(&self) -> usize {
        if self.inner.deleted.load(Ordering::Acquire) {
            return 0;
        }
        self.inner.data.read().id_to_slot.len()
    }

    /// `true` when the collection holds no live vectors.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Current collection name (renames update it — CORE-022).
    pub fn name(&self) -> String {
        self.inner.name.read().clone()
    }

    /// Rebuild the HNSW index from scratch over the live vector set,
    /// purging tombstoned slots from the graph, and recompute quantization
    /// codes over the same live set (SPEC-001 CORE-032). Quantization is
    /// always batch-fit here — never per-upsert — for byte-identical
    /// parity with a fresh ingest (CORE-041).
    pub fn reindex(&self) -> Result<()> {
        self.guard()?;
        let mut data = self.inner.data.write();
        let dim = self.inner.config.dimension;

        let mut live: Vec<(usize, Vec<f32>)> = Vec::new();
        for (slot, id) in data.ids.iter().enumerate() {
            if data.id_to_slot.get(id) == Some(&slot) {
                live.push((slot, data.vectors[slot * dim..(slot + 1) * dim].to_vec()));
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            data.index = if self.inner.config.metric == Metric::DotProduct {
                None
            } else {
                let fresh = HnswIndex::new(
                    self.inner.config.metric,
                    dim,
                    self.inner.config.hnsw.m,
                    self.inner.config.hnsw.ef_construction,
                    live.len().max(1),
                )?;
                if !live.is_empty() {
                    fresh.insert_batch(&live);
                }
                Some(fresh)
            };
        }

        recompute_quantization(&mut data, &self.inner.config, &live)?;
        Ok(())
    }

    /// k-NN search over the dense index, ordered per metric (CORE-035):
    /// descending similarity for Cosine/DotProduct, ascending distance for
    /// Euclidean. Payload is included, the stored vector is not (SPEC-004 §4
    /// defaults); use [`query`](Self::query) to override.
    pub fn search(&self, vector: &[f32], limit: usize) -> Result<Vec<Hit>> {
        self.execute_query(vector, limit, None, true, false, None)
    }

    /// Start a fluent query with per-query overrides (SPEC-004 §5). The builder
    /// holds no lock until `run` (API-030).
    pub fn query<'a>(&'a self, vector: &'a [f32]) -> QueryBuilder<'a> {
        QueryBuilder::new(self, vector)
    }

    /// Sparse dot-product search over the BYO sparse lane (SPEC-007 HYB-003):
    /// score each live point's sparse vector against `query`, ordered by score
    /// descending, ties broken by id (bytewise). Only non-zero matches are
    /// returned.
    pub fn search_sparse(&self, query: &SparseVector, limit: usize) -> Result<Vec<Hit>> {
        self.guard()?;
        if limit == 0 {
            return Err(VecLiteError::InvalidArgument(
                "limit must be greater than 0".to_owned(),
            ));
        }
        query.validate()?;
        let ranked = self.sparse_ranked(query, limit, None);
        let data = self.inner.data.read();
        Ok(project_slots(
            &data,
            &ranked,
            self.inner.config.dimension,
            true,
            false,
        ))
    }

    /// Start a fluent hybrid query fusing the dense and sparse lanes with RRF
    /// (SPEC-007 §2–3). The builder holds no lock until `run`.
    pub fn hybrid_query(&self) -> crate::hybrid::HybridQuery<'_> {
        crate::hybrid::HybridQuery::new(self)
    }

    /// Ranked `(slot, score)` for the sparse lane, filtered and truncated to
    /// `limit`, deterministic (score desc, then id). Shared by `search_sparse`
    /// and the hybrid fuser.
    fn sparse_ranked(
        &self,
        query: &SparseVector,
        limit: usize,
        filter: Option<&Filter>,
    ) -> Vec<(usize, f32)> {
        let data = self.inner.data.read();
        let mut scored: Vec<(usize, f32)> = Vec::new();
        for slot in 0..data.ids.len() {
            if data.id_to_slot.get(&data.ids[slot]) != Some(&slot) {
                continue;
            }
            if let Some(f) = filter {
                if !f.matches(data.payloads[slot].as_ref()) {
                    continue;
                }
            }
            if let Some(sp) = &data.sparses[slot] {
                let score = sp.dot(query);
                if score != 0.0 {
                    scored.push((slot, score));
                }
            }
        }
        scored.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| data.ids[a.0].cmp(&data.ids[b.0]))
        });
        scored.truncate(limit);
        scored
    }

    /// Execute a hybrid query (SPEC-007 §3). A single provided lane degenerates
    /// to that lane's plain search with its own scores (HYB-010); two lanes are
    /// fused with reciprocal rank fusion (HYB-020/021).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_hybrid(
        &self,
        dense: Option<&[f32]>,
        sparse: Option<&SparseVector>,
        alpha: f32,
        rrf_k: f32,
        limit: usize,
        with_payload: bool,
        with_vector: bool,
        filter: Option<&Filter>,
    ) -> Result<Vec<Hit>> {
        self.guard()?;
        if limit == 0 {
            return Err(VecLiteError::InvalidArgument(
                "limit must be greater than 0".to_owned(),
            ));
        }
        if let Some(s) = sparse {
            s.validate()?;
        }
        match (dense, sparse) {
            (None, None) => Err(VecLiteError::InvalidArgument(
                "a hybrid query needs at least one lane (dense or sparse)".to_owned(),
            )),
            // Degenerate: exactly one lane → that lane's plain search (HYB-010).
            (Some(d), None) => {
                self.execute_query(d, limit, None, with_payload, with_vector, filter)
            }
            (None, Some(s)) => {
                let ranked = self.sparse_ranked(s, limit, filter);
                let data = self.inner.data.read();
                Ok(project_slots(
                    &data,
                    &ranked,
                    self.inner.config.dimension,
                    with_payload,
                    with_vector,
                ))
            }
            (Some(d), Some(s)) => {
                let fetch = limit.saturating_mul(4).max(100);
                // Dense lane ranked ids (projection off — we only need the order).
                let dense_hits = self.execute_query(d, fetch, None, false, false, filter)?;
                let dense_rank: HashMap<&str, usize> = dense_hits
                    .iter()
                    .enumerate()
                    .map(|(i, h)| (h.id.as_str(), i + 1))
                    .collect();
                // Sparse lane ranked ids.
                let sparse_slots = self.sparse_ranked(s, fetch, filter);
                let data = self.inner.data.read();
                let sparse_rank: HashMap<&str, usize> = sparse_slots
                    .iter()
                    .enumerate()
                    .map(|(i, (slot, _))| (data.ids[*slot].as_str(), i + 1))
                    .collect();

                // Fuse over the union of ids (HYB-020).
                let mut ids: Vec<&str> = dense_rank
                    .keys()
                    .chain(sparse_rank.keys())
                    .copied()
                    .collect();
                ids.sort_unstable();
                ids.dedup();

                let mut fused: Vec<(&str, f32, usize)> = ids
                    .into_iter()
                    .map(|id| {
                        let dr = dense_rank.get(id).copied();
                        let sr = sparse_rank.get(id).copied();
                        let d_term = dr.map_or(0.0, |r| alpha / (rrf_k + r as f32));
                        let s_term = sr.map_or(0.0, |r| (1.0 - alpha) / (rrf_k + r as f32));
                        (id, d_term + s_term, dr.unwrap_or(usize::MAX))
                    })
                    .collect();
                // Order by fused score desc, ties by dense rank asc, then id (HYB-021).
                fused.sort_by(|a, b| {
                    b.1.total_cmp(&a.1)
                        .then_with(|| a.2.cmp(&b.2))
                        .then_with(|| a.0.cmp(b.0))
                });
                fused.truncate(limit);

                // Project the fused ids from the live data.
                let dim = self.inner.config.dimension;
                let hits = fused
                    .into_iter()
                    .filter_map(|(id, score, _)| {
                        data.id_to_slot.get(id).map(|&slot| Hit {
                            id: id.to_owned(),
                            score,
                            payload: if with_payload {
                                data.payloads[slot].clone()
                            } else {
                                None
                            },
                            vector: if with_vector {
                                Some(data.vectors[slot * dim..(slot + 1) * dim].to_vec())
                            } else {
                                None
                            },
                        })
                    })
                    .collect();
                Ok(hits)
            }
        }
    }

    /// Shared search behind `search` and `QueryBuilder::run`. Validates inputs,
    /// normalizes the query for Cosine (CORE-014), gathers candidates from the
    /// HNSW index when present (over-fetching past tombstones) or by exact
    /// brute force otherwise (DotProduct, or any metric on wasm32), scores and
    /// orders per CORE-035, and projects to `Hit`.
    pub(crate) fn execute_query(
        &self,
        query: &[f32],
        limit: usize,
        ef_search: Option<usize>,
        with_payload: bool,
        with_vector: bool,
        filter: Option<&Filter>,
    ) -> Result<Vec<Hit>> {
        self.guard()?;
        // Auto-embed collections rebuild a stale vocabulary before any search so
        // stored vectors and the query share one vocabulary (SPEC-005 §5).
        self.refit_if_dirty()?;
        // Reject unsupported filter features up front (geo, nested-path keys —
        // FLT-012), even for an otherwise-empty filter.
        if let Some(f) = filter {
            f.validate()?;
        }
        if limit == 0 {
            return Err(VecLiteError::InvalidArgument(
                "limit must be greater than 0".to_owned(),
            ));
        }
        let dim = self.inner.config.dimension;
        if query.len() != dim {
            return Err(VecLiteError::DimensionMismatch {
                expected: dim,
                got: query.len(),
            });
        }
        if query.iter().any(|v| !v.is_finite()) {
            return Err(VecLiteError::InvalidArgument(
                "query vector contains NaN or infinite values".to_owned(),
            ));
        }
        let ef_search = ef_search.unwrap_or(self.inner.config.hnsw.ef_search);
        if !EF_SEARCH_BOUNDS.contains(&ef_search) {
            return Err(VecLiteError::InvalidArgument(format!(
                "ef_search must be in {}..={}, got {ef_search}",
                EF_SEARCH_BOUNDS.start(),
                EF_SEARCH_BOUNDS.end()
            )));
        }
        let metric = self.inner.config.metric;

        // Cosine normalizes the query at search time (CORE-014).
        let normalized;
        let q: &[f32] = if metric == Metric::Cosine {
            let norm = query
                .iter()
                .map(|v| f64::from(*v) * f64::from(*v))
                .sum::<f64>()
                .sqrt();
            if norm == 0.0 {
                return Err(VecLiteError::InvalidArgument(
                    "zero query vector is not allowed with the cosine metric".to_owned(),
                ));
            }
            #[allow(clippy::cast_possible_truncation)]
            {
                normalized = query
                    .iter()
                    .map(|v| (f64::from(*v) / norm) as f32)
                    .collect::<Vec<f32>>();
            }
            &normalized
        } else {
            query
        };

        let data = self.inner.data.read();
        if data.id_to_slot.is_empty() {
            return Ok(Vec::new());
        }

        let mut scored: Vec<(usize, f32)> = Vec::new();
        // An empty filter matches everything, so it takes the fast (index)
        // path; only a filter with at least one clause diverts to the exact
        // filtered path (SPEC-006 §4).
        let active =
            filter.filter(|f| !(f.must.is_empty() && f.should.is_empty() && f.must_not.is_empty()));
        match active {
            Some(f) => filtered_scored(&data, q, metric, dim, f, &mut scored),
            None => {
                #[cfg(not(target_arch = "wasm32"))]
                match &data.index {
                    // For a small live set (no more points than we are fetching),
                    // the HNSW approximation can drop the farthest candidates;
                    // exact brute force is both correct and cheaper. Larger sets
                    // use the index.
                    Some(_) if data.id_to_slot.len() <= limit + data.tombstones.len() => {
                        brute_force(&data, q, metric, dim, &mut scored);
                    }
                    Some(index) => {
                        let fetch = limit + data.tombstones.len();
                        for (slot, distance) in index.search(q, fetch, ef_search)? {
                            if data.id_to_slot.get(&data.ids[slot]) == Some(&slot) {
                                scored.push((slot, distance_to_score(metric, distance)));
                                if scored.len() == limit {
                                    break;
                                }
                            }
                        }
                        // HNSW is approximate and can under-return; if it yielded
                        // fewer than the available results, fall back to exact so
                        // `search` always returns `min(limit, live)` (CORE-035).
                        if scored.len() < limit.min(data.id_to_slot.len()) {
                            scored.clear();
                            brute_force(&data, q, metric, dim, &mut scored);
                        }
                    }
                    None => brute_force(&data, q, metric, dim, &mut scored),
                }
                #[cfg(target_arch = "wasm32")]
                brute_force(&data, q, metric, dim, &mut scored);
            }
        }

        // Order per CORE-035, then truncate to the limit.
        if metric_is_similarity(metric) {
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        } else {
            scored.sort_by(|a, b| a.1.total_cmp(&b.1));
        }
        scored.truncate(limit);

        let hits = scored
            .into_iter()
            .map(|(slot, score)| Hit {
                id: data.ids[slot].clone(),
                score,
                payload: if with_payload {
                    data.payloads[slot].clone()
                } else {
                    None
                },
                vector: if with_vector {
                    Some(data.vectors[slot * dim..(slot + 1) * dim].to_vec())
                } else {
                    None
                },
            })
            .collect();
        Ok(hits)
    }
}

/// Persistence hooks (native-only): WAL logging around writes, the live-point
/// snapshot a checkpoint seals, and the replay paths recovery drives.
#[cfg(not(target_arch = "wasm32"))]
impl Collection {
    /// Encode an upsert batch for the WAL, or `None` for a memory collection.
    fn encode_wal(&self, points: &[Point]) -> Result<Option<Vec<u8>>> {
        if self.inner.persistence.is_some() {
            Ok(Some(wal_body::encode(points)?))
        } else {
            Ok(None)
        }
    }

    /// Append a WAL entry when a body was produced (persistent collection).
    fn log(&self, op: WalOp, body: Option<Vec<u8>>) -> Result<()> {
        if let (Some(p), Some(body)) = (&self.inner.persistence, body) {
            p.log(self.inner.coll_id, op.to_byte(), body)?;
        }
        Ok(())
    }

    /// Drive a checkpoint if the WAL crossed its threshold (WAL-030a).
    fn after_write(&self) -> Result<()> {
        if let Some(p) = &self.inner.persistence {
            p.after_write()?;
        }
        Ok(())
    }

    /// Live points `(id, dense vector, payload)` in slot order — the input a
    /// checkpoint seals (sparse not yet persisted, phase3c).
    pub(crate) fn live_points(&self) -> Vec<crate::persist::seal::LivePoint> {
        let data = self.inner.data.read();
        let dim = self.inner.config.dimension;
        let mut out = Vec::with_capacity(data.id_to_slot.len());
        for slot in 0..data.ids.len() {
            if data.id_to_slot.get(&data.ids[slot]) == Some(&slot) {
                out.push((
                    data.ids[slot].clone(),
                    data.vectors[slot * dim..(slot + 1) * dim].to_vec(),
                    data.payloads[slot].clone(),
                ));
            }
        }
        out
    }

    /// Apply a WAL upsert during recovery — validate + apply, no re-logging.
    /// Reserved keys are allowed: recovered text documents carry `_text`.
    pub(crate) fn replay_upsert(&self, points: Vec<Point>) -> Result<()> {
        let prepared: Vec<PreparedPoint> = points
            .into_iter()
            .map(|p| self.prepare_inner(p, true))
            .collect::<Result<_>>()?;
        self.apply_prepared(prepared);
        Ok(())
    }

    /// Apply a WAL delete during recovery.
    pub(crate) fn replay_delete(&self, ids: &[String]) {
        let mut data = self.inner.data.write();
        for id in ids {
            tombstone(&mut data, id);
        }
    }

    /// Fraction of allocated slots that are tombstoned (dead), in `0.0..1.0`;
    /// 0.0 when the collection has no slots. Drives auto-vacuum escalation
    /// (SPEC-002 STG-072).
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn tombstone_ratio(&self) -> f32 {
        let data = self.inner.data.read();
        let total = data.ids.len();
        if total == 0 {
            return 0.0;
        }
        data.tombstones.len() as f32 / total as f32
    }

    /// Reclaim tombstoned slots in memory (SPEC-002 STG-071): rebuild the slot
    /// storage and HNSW graph over only the live points, renumbering slots to
    /// `0..live`. Clears the tombstone set so a following checkpoint seals a
    /// minimal set and auto-vacuum does not immediately re-trigger. Stored
    /// vectors are already validated/normalized, so no re-preparation is needed.
    pub(crate) fn compact(&self) -> Result<()> {
        self.guard()?;
        let mut data = self.inner.data.write();
        let dim = self.inner.config.dimension;

        // Snapshot live points in slot order before resetting storage.
        let mut live: Vec<PreparedPoint> = Vec::with_capacity(data.id_to_slot.len());
        for slot in 0..data.ids.len() {
            if data.id_to_slot.get(&data.ids[slot]) == Some(&slot) {
                live.push(PreparedPoint {
                    id: data.ids[slot].clone(),
                    vector: data.vectors[slot * dim..(slot + 1) * dim].to_vec(),
                    sparse: data.sparses[slot].clone(),
                    payload: data.payloads[slot].clone(),
                });
            }
        }

        data.vectors.clear();
        data.ids.clear();
        data.id_to_slot.clear();
        data.tombstones.clear();
        data.payloads.clear();
        data.sparses.clear();
        // Slots are renumbered, so the payload indexes are rebuilt from scratch
        // (apply_upsert below re-inserts each live point at its fresh slot).
        data.payload_indexes = PayloadIndexes::new(&self.inner.config.payload_indexes);
        data.index = if self.inner.config.metric == Metric::DotProduct {
            None
        } else {
            Some(HnswIndex::new(
                self.inner.config.metric,
                dim,
                self.inner.config.hnsw.m,
                self.inner.config.hnsw.ef_construction,
                live.len().max(1),
            )?)
        };

        // The quantization refit reads the live set at its fresh slot numbers.
        let live_for_quant: Vec<(usize, Vec<f32>)> = live
            .iter()
            .enumerate()
            .map(|(slot, p)| (slot, p.vector.clone()))
            .collect();
        for p in live {
            apply_upsert(&mut data, p, dim);
        }
        recompute_quantization(&mut data, &self.inner.config, &live_for_quant)
    }
}

/// True for metrics whose score is a similarity (higher = closer, sorted
/// descending); false for distance metrics sorted ascending (CORE-035).
fn metric_is_similarity(metric: Metric) -> bool {
    matches!(metric, Metric::Cosine | Metric::DotProduct)
}

/// Convert a raw hnsw_rs distance to the metric's score (CORE-035). Only the
/// index-backed metrics reach this: Cosine (`DistCosine = 1 - cos`, so the
/// similarity is `1 - distance`) and Euclidean (`DistL2` is the L2 distance,
/// used as the score directly).
#[cfg(not(target_arch = "wasm32"))]
fn distance_to_score(metric: Metric, distance: f32) -> f32 {
    match metric {
        Metric::Cosine => 1.0 - distance,
        _ => distance,
    }
}

/// Exact search over every live slot, used when there is no HNSW index:
/// DotProduct on native, and all metrics on wasm32. Scores are computed with
/// the same semantics as the index paths so results stay consistent.
fn brute_force(
    data: &CollectionData,
    query: &[f32],
    metric: Metric,
    dim: usize,
    out: &mut Vec<(usize, f32)>,
) {
    for slot in 0..data.ids.len() {
        if data.id_to_slot.get(&data.ids[slot]) != Some(&slot) {
            continue; // tombstoned or replaced by a newer slot
        }
        out.push((slot, score_slot(data, query, metric, dim, slot)));
    }
}

/// Project ranked `(slot, score)` pairs into `Hit`s (SPEC-004 §4 projection).
fn project_slots(
    data: &CollectionData,
    ranked: &[(usize, f32)],
    dim: usize,
    with_payload: bool,
    with_vector: bool,
) -> Vec<Hit> {
    ranked
        .iter()
        .map(|&(slot, score)| Hit {
            id: data.ids[slot].clone(),
            score,
            payload: if with_payload {
                data.payloads[slot].clone()
            } else {
                None
            },
            vector: if with_vector {
                Some(data.vectors[slot * dim..(slot + 1) * dim].to_vec())
            } else {
                None
            },
        })
        .collect()
}

/// Score one slot's stored vector against the query, per metric (CORE-035).
fn score_slot(
    data: &CollectionData,
    query: &[f32],
    metric: Metric,
    dim: usize,
    slot: usize,
) -> f32 {
    let v = &data.vectors[slot * dim..(slot + 1) * dim];
    match metric {
        Metric::Cosine => cosine_similarity(query, v),
        Metric::Euclidean => euclidean_distance(query, v),
        Metric::DotProduct => dot_product(query, v),
    }
}

/// Exact filtered scoring (SPEC-006 §4). When a payload index yields a candidate
/// superset for the `must` clause, only those slots are considered (pre-filter);
/// otherwise every live slot is scanned (post-filter). Either way the **full**
/// filter is applied to each slot, so the result set is identical to a scan
/// (FLT-022/031). Exact brute-force scoring keeps all metrics correct.
fn filtered_scored(
    data: &CollectionData,
    query: &[f32],
    metric: Metric,
    dim: usize,
    filter: &Filter,
    out: &mut Vec<(usize, f32)>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(candidates) = data.payload_indexes.candidates(filter) {
        for s in candidates.iter() {
            let slot = s as usize;
            if slot < data.ids.len()
                && data.id_to_slot.get(&data.ids[slot]) == Some(&slot)
                && filter.matches(data.payloads[slot].as_ref())
            {
                out.push((slot, score_slot(data, query, metric, dim, slot)));
            }
        }
        return;
    }
    for slot in 0..data.ids.len() {
        if data.id_to_slot.get(&data.ids[slot]) == Some(&slot)
            && filter.matches(data.payloads[slot].as_ref())
        {
            out.push((slot, score_slot(data, query, metric, dim, slot)));
        }
    }
}

/// Tombstone the slot behind `id`, clearing its payload/sparse storage.
/// Returns whether the id existed.
fn tombstone(data: &mut CollectionData, id: &str) -> bool {
    match data.id_to_slot.remove(id) {
        Some(slot) => {
            data.tombstones.insert(slot);
            #[cfg(not(target_arch = "wasm32"))]
            data.payload_indexes
                .remove(slot as u64, data.payloads[slot].as_ref());
            data.payloads[slot] = None;
            data.sparses[slot] = None;
            true
        }
        None => false,
    }
}

/// Append-mostly upsert: an existing id gets its old slot tombstoned and the
/// new value a fresh slot (CORE-033 pairs this with HNSW soft-deletes).
fn apply_upsert(data: &mut CollectionData, p: PreparedPoint, dim: usize) {
    if let Some(slot) = data.id_to_slot.remove(&p.id) {
        data.tombstones.insert(slot);
        #[cfg(not(target_arch = "wasm32"))]
        data.payload_indexes
            .remove(slot as u64, data.payloads[slot].as_ref());
        data.payloads[slot] = None;
        data.sparses[slot] = None;
    }
    let slot = data.ids.len();
    debug_assert_eq!(data.vectors.len(), slot * dim);
    data.vectors.extend_from_slice(&p.vector);
    data.ids.push(p.id.clone());
    data.payloads.push(p.payload);
    data.sparses.push(p.sparse);
    data.id_to_slot.insert(p.id, slot);
    #[cfg(not(target_arch = "wasm32"))]
    data.payload_indexes
        .insert(slot as u64, data.payloads[slot].as_ref());

    // The replaced-id path above already tombstoned the old slot; hnsw_rs
    // has no delete API, so that stale node stays in the graph until
    // `reindex` — the search layer filters it out by liveness instead.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(index) = &data.index {
        index.insert(&data.vectors[slot * dim..(slot + 1) * dim], slot);
    }
}

/// Recompute quantization codes over the live vector set (batch-fit, never
/// per-upsert, for byte-identical parity — CORE-041). Clears `codes` when
/// there are no live vectors or `Quantization::None` is configured.
fn recompute_quantization(
    data: &mut CollectionData,
    config: &CollectionOptions,
    live: &[(usize, Vec<f32>)],
) -> Result<()> {
    if live.is_empty() {
        data.codes.clear();
        data.quant_params = None;
        return Ok(());
    }
    let vectors: Vec<Vec<f32>> = live.iter().map(|(_, v)| v.clone()).collect();
    match config.quantization {
        Quantization::None => {
            data.codes.clear();
            data.quant_params = None;
        }
        Quantization::Scalar { bits } => {
            let mut sq = ScalarQuantization::new(bits)
                .map_err(|e| VecLiteError::InvalidArgument(e.to_string()))?;
            sq.fit(&vectors)
                .map_err(|e| VecLiteError::InvalidArgument(e.to_string()))?;
            let quantized = sq
                .quantize(&vectors)
                .map_err(|e| VecLiteError::InvalidArgument(e.to_string()))?;
            data.quant_params = Some(
                sq.serialize_params()
                    .map_err(|e| VecLiteError::InvalidArgument(e.to_string()))?,
            );
            data.codes = quantized.data;
        }
        Quantization::Binary => {
            let mut bq = BinaryQuantization::new();
            bq.train(&vectors)
                .map_err(|e| VecLiteError::InvalidArgument(e.to_string()))?;
            let quantized = bq
                .quantize(&vectors)
                .map_err(|e| VecLiteError::InvalidArgument(e.to_string()))?;
            data.quant_params = Some(
                bq.serialize_params()
                    .map_err(|e| VecLiteError::InvalidArgument(e.to_string()))?,
            );
            data.codes = quantized.data;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coll(dimension: usize, metric: Metric) -> Collection {
        Collection {
            inner: Arc::new(
                CollectionInner::new(
                    "t".into(),
                    CollectionOptions::new(dimension, metric).quantization(Quantization::None),
                    0,
                    None,
                )
                .unwrap_or_else(|e| panic!("{e}")),
            ),
        }
    }

    #[test]
    fn upsert_get_roundtrip() {
        let c = coll(3, Metric::Euclidean);
        c.upsert(Point::new("a", vec![1.0, 2.0, 3.0]).payload(serde_json::json!({"k": 1})))
            .unwrap_or_else(|e| panic!("{e}"));
        let p = c
            .get("a")
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("present"));
        assert_eq!(p.vector, vec![1.0, 2.0, 3.0]);
        assert_eq!(p.payload, Some(serde_json::json!({"k": 1})));
        assert_eq!(c.len(), 1);
        assert!(c.get("missing").unwrap_or_else(|e| panic!("{e}")).is_none());
    }

    #[test]
    fn dimension_mismatch_leaves_state_unchanged() {
        let c = coll(3, Metric::Euclidean);
        let Err(err) = c.upsert(Point::new("a", vec![1.0])) else {
            panic!("must fail")
        };
        assert!(matches!(
            err,
            VecLiteError::DimensionMismatch {
                expected: 3,
                got: 1
            }
        ));
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn non_finite_vectors_rejected() {
        let c = coll(2, Metric::Euclidean);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let Err(err) = c.upsert(Point::new("a", vec![1.0, bad])) else {
                panic!("must fail")
            };
            assert!(matches!(err, VecLiteError::InvalidArgument(_)));
        }
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn cosine_normalizes_at_ingest_and_rejects_zero() {
        let c = coll(2, Metric::Cosine);
        c.upsert(Point::new("a", vec![3.0, 4.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        let p = c
            .get("a")
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("present"));
        assert!((p.vector[0] - 0.6).abs() < 1e-6);
        assert!((p.vector[1] - 0.8).abs() < 1e-6);

        let Err(err) = c.upsert(Point::new("z", vec![0.0, 0.0])) else {
            panic!("zero vector must fail")
        };
        assert!(matches!(err, VecLiteError::InvalidArgument(_)));
    }

    #[test]
    fn upsert_replaces_existing_id() {
        let c = coll(2, Metric::Euclidean);
        c.upsert(Point::new("a", vec![1.0, 1.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("a", vec![2.0, 2.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.len(), 1);
        let p = c
            .get("a")
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("present"));
        assert_eq!(p.vector, vec![2.0, 2.0]);
    }

    #[test]
    fn delete_semantics() {
        let c = coll(1, Metric::Euclidean);
        c.upsert(Point::new("a", vec![1.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(c.delete("a").unwrap_or_else(|e| panic!("{e}")));
        assert!(!c.delete("a").unwrap_or_else(|e| panic!("{e}")));
        assert_eq!(c.len(), 0);
        assert!(c.get("a").unwrap_or_else(|e| panic!("{e}")).is_none());

        c.upsert_batch(vec![Point::new("x", vec![1.0]), Point::new("y", vec![2.0])])
            .unwrap_or_else(|e| panic!("{e}"));
        let n = c
            .delete_batch(&["x", "y", "missing"])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(n, 2);
        assert!(c.is_empty());
    }

    #[test]
    fn failed_batch_applies_nothing() {
        let c = coll(2, Metric::Euclidean);
        let Err(err) = c.upsert_batch(vec![
            Point::new("ok", vec![1.0, 2.0]),
            Point::new("bad", vec![1.0]), // wrong dimension
        ]) else {
            panic!("must fail")
        };
        assert!(matches!(err, VecLiteError::DimensionMismatch { .. }));
        assert_eq!(c.len(), 0);
        assert!(c.get("ok").unwrap_or_else(|e| panic!("{e}")).is_none());
    }

    #[test]
    fn empty_id_rejected() {
        let c = coll(1, Metric::Euclidean);
        let Err(err) = c.upsert(Point::new("", vec![1.0])) else {
            panic!("must fail")
        };
        assert!(matches!(err, VecLiteError::InvalidArgument(_)));
    }
}
