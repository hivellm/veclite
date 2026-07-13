//! Collection handle and in-memory slot storage (SPEC-001 §3–4).
//!
//! Storage is append-mostly, mirroring the on-disk design (SPEC-002 STG-002):
//! vectors live in a flat slot-major block; updates and deletes tombstone the
//! old slot and appends take a fresh one. Space reclamation is `vacuum`'s job
//! (phase2d); the HNSW index (phase1b) shares the same slot numbering.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::RwLock;

use crate::error::{Result, VecLiteError};
#[cfg(not(target_arch = "wasm32"))]
use crate::index::HnswIndex;
use crate::options::{CollectionOptions, Metric, Quantization};
use crate::point::{Point, SparseVector, validate_id};
use crate::quantization::traits::{QuantizationMethod, QuantizationParams};
use crate::quantization::{BinaryQuantization, ScalarQuantization};

/// Shared state behind every [`Collection`] handle for one collection.
pub(crate) struct CollectionInner {
    /// Current name; renames update it in place (CORE-022).
    pub(crate) name: RwLock<String>,
    /// Immutable configuration (CORE-016).
    pub(crate) config: CollectionOptions,
    /// Set by `delete_collection`; stale handles then fail with
    /// `CollectionNotFound` (CORE-021).
    pub(crate) deleted: AtomicBool,
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
}

impl CollectionData {
    #[cfg(not(target_arch = "wasm32"))]
    fn empty(index: Option<HnswIndex>) -> Self {
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

/// Allocation hint handed to a freshly created HNSW graph (native only).
/// Not a hard cap — hnsw_rs reallocates past it (SPEC-001 CORE-030).
#[cfg(not(target_arch = "wasm32"))]
const INITIAL_INDEX_CAPACITY: usize = 1024;

impl CollectionInner {
    /// Build a new collection's shared state, including its HNSW index
    /// (native only). Propagates `HnswIndex::new`'s `m`/`ef_construction`
    /// bounds check (SPEC-001 CORE-031); `DotProduct` collections get no
    /// index (ADR-0002 — `DistDot` panics on unnormalized vectors).
    pub(crate) fn new(name: String, config: CollectionOptions) -> Result<Self> {
        #[cfg(not(target_arch = "wasm32"))]
        let data = {
            let index = match config.metric {
                Metric::DotProduct => None,
                _ => Some(HnswIndex::new(
                    config.metric,
                    config.dimension,
                    config.hnsw.m,
                    config.hnsw.ef_construction,
                    INITIAL_INDEX_CAPACITY,
                )?),
            };
            CollectionData::empty(index)
        };
        #[cfg(target_arch = "wasm32")]
        let data = CollectionData::empty();

        Ok(CollectionInner {
            name: RwLock::new(name),
            deleted: AtomicBool::new(false),
            data: RwLock::new(data),
            config,
        })
    }
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

    /// Validate ingest rules (CORE-010, CORE-012..014) and normalize for
    /// Cosine. Runs outside any lock; on error nothing was modified.
    fn prepare(&self, point: Point) -> Result<PreparedPoint> {
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
        self.guard()?;
        let prepared = self.prepare(point)?;
        let mut data = self.inner.data.write();
        apply_upsert(&mut data, prepared, self.inner.config.dimension);
        Ok(())
    }

    /// Insert-or-replace a batch. The batch is the atomic unit: every point
    /// is validated before any is applied, and all become visible together
    /// (SPEC-003 WAL-012 semantics).
    pub fn upsert_batch(&self, points: Vec<Point>) -> Result<()> {
        self.guard()?;
        let prepared: Vec<PreparedPoint> = points
            .into_iter()
            .map(|p| self.prepare(p))
            .collect::<Result<_>>()?;
        let mut data = self.inner.data.write();
        for p in prepared {
            apply_upsert(&mut data, p, self.inner.config.dimension);
        }
        Ok(())
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
        self.guard()?;
        let mut data = self.inner.data.write();
        Ok(tombstone(&mut data, id))
    }

    /// Delete a batch of ids; returns how many existed. One atomic unit,
    /// like `upsert_batch`.
    pub fn delete_batch(&self, ids: &[&str]) -> Result<usize> {
        self.guard()?;
        let mut data = self.inner.data.write();
        Ok(ids.iter().filter(|id| tombstone(&mut data, id)).count())
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

    /// Query the native HNSW index and filter out tombstoned slots,
    /// over-fetching to compensate for hnsw_rs having no delete API. Returns
    /// raw hnsw distances with no score transformation — ordering and `Hit`
    /// mapping are the public `search`'s job (phase1c, CORE-035).
    // Not yet called: the public `search`/`Hit` API lands in phase1c, which
    // consumes this. Allowed dead-code until then, same as the other
    // not-yet-wired internals in this crate (see lib.rs `index`/`quantization`).
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(dead_code)]
    pub(crate) fn search_internal(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<(String, f32)>> {
        self.guard()?;
        let data = self.inner.data.read();
        let Some(index) = &data.index else {
            return Err(VecLiteError::InvalidArgument(
                "collection has no HNSW index (DotProduct indexing is not yet supported)"
                    .to_owned(),
            ));
        };
        let fetch = k + data.tombstones.len();
        let hits = index.search(query, fetch, ef_search)?;
        let mut results = Vec::with_capacity(k.min(hits.len()));
        for (slot, distance) in hits {
            if data.id_to_slot.get(&data.ids[slot]) == Some(&slot) {
                results.push((data.ids[slot].clone(), distance));
                if results.len() == k {
                    break;
                }
            }
        }
        Ok(results)
    }
}

/// Tombstone the slot behind `id`, clearing its payload/sparse storage.
/// Returns whether the id existed.
fn tombstone(data: &mut CollectionData, id: &str) -> bool {
    match data.id_to_slot.remove(id) {
        Some(slot) => {
            data.tombstones.insert(slot);
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
