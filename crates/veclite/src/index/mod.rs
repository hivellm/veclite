//! HNSW index wrapper. Adapted from hivellm/vectorizer vectorizer@3.5.0
//! (crates/vectorizer/src/db/optimized_hnsw.rs), Apache-2.0. Generalized to
//! three metrics; async/GPU/shard/file-dump branches removed. Native-only
//! (ADR-0002): hnsw_rs cannot build on wasm32.
//!
//! Node ids are the collection's slot numbers (SPEC-001 CORE-032/033): the
//! collection assigns each stored vector a slot and inserts `(&vector, slot)`
//! here. Deletes are soft — hnsw_rs 0.3.x has no delete API, so the collection
//! filters tombstoned slots out of search results and over-fetches to
//! compensate. `search` returns raw `(slot, distance)` pairs with no liveness
//! filtering and no score transformation; that is the query layer's job.

use hnsw_rs::prelude::{DistCosine, DistDot, DistL2, Hnsw, Neighbour};

use crate::error::{Result, VecLiteError};
use crate::options::Metric;

/// HNSW connectivity bounds (SPEC-001 CORE-031).
const M_BOUNDS: std::ops::RangeInclusive<usize> = 4..=64;
/// Construction candidate-list bounds (CORE-031).
const EF_CONSTRUCTION_BOUNDS: std::ops::RangeInclusive<usize> = 8..=2048;
/// Per-query candidate-list bounds (CORE-031).
const EF_SEARCH_BOUNDS: std::ops::RangeInclusive<usize> = 1..=4096;

/// One monomorphized `Hnsw` per metric. hnsw_rs takes the distance as a type
/// parameter, so the metric choice is a static-dispatch enum — no vtable, and
/// the closed metric set makes this exhaustive.
///
/// Metric → distance (anndists 0.1.3 semantics, all "lower is closer"):
/// - `Cosine` → `DistCosine`, `eval = 1 - cos`. Vectors are unit-normalized at
///   ingest (CORE-014), matching the server's cosine path exactly.
/// - `Euclidean` → `DistL2`, squared-L2 style distance.
/// - `DotProduct` → `DistDot`, `eval = 1 - dot`. NOTE: anndists asserts
///   `dot <= 1`, so `DistDot` panics on vectors whose inner product exceeds 1
///   (i.e. unnormalized inputs). The collection therefore does not build a
///   `DotProduct` HNSW index yet (tracked as a follow-up); this variant is
///   safe only for bounded/normalized inputs and is exercised as such in tests.
enum Graph {
    Cosine(Hnsw<'static, f32, DistCosine>),
    L2(Hnsw<'static, f32, DistL2>),
    Dot(Hnsw<'static, f32, DistDot>),
}

/// A metric-typed HNSW graph keyed by collection slot number.
pub(crate) struct HnswIndex {
    graph: Graph,
}

impl HnswIndex {
    /// Build an empty index. `capacity` is hnsw_rs's allocation hint (not a
    /// hard cap — exceeding it only costs a reallocation). Rejects out-of-range
    /// `m`/`ef_construction` with `InvalidArgument` (CORE-031).
    pub(crate) fn new(
        metric: Metric,
        dimension: usize,
        m: usize,
        ef_construction: usize,
        capacity: usize,
    ) -> Result<Self> {
        if !M_BOUNDS.contains(&m) {
            return Err(VecLiteError::InvalidArgument(format!(
                "hnsw m must be in {}..={}, got {m}",
                M_BOUNDS.start(),
                M_BOUNDS.end()
            )));
        }
        if !EF_CONSTRUCTION_BOUNDS.contains(&ef_construction) {
            return Err(VecLiteError::InvalidArgument(format!(
                "hnsw ef_construction must be in {}..={}, got {ef_construction}",
                EF_CONSTRUCTION_BOUNDS.start(),
                EF_CONSTRUCTION_BOUNDS.end()
            )));
        }
        let _ = dimension; // hnsw_rs infers dimension from the first insert.
        // Layer count follows the server's heuristic; clamp to a valid range.
        // hnsw_rs internally caps this at NB_LAYER_MAX (16).
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let max_layer = 16.min((capacity.max(2) as f32).ln() as usize).max(1);
        let max_elements = capacity.max(1);
        let graph = match metric {
            Metric::Cosine => Graph::Cosine(Hnsw::new(
                m,
                max_elements,
                max_layer,
                ef_construction,
                DistCosine {},
            )),
            Metric::Euclidean => Graph::L2(Hnsw::new(
                m,
                max_elements,
                max_layer,
                ef_construction,
                DistL2 {},
            )),
            Metric::DotProduct => Graph::Dot(Hnsw::new(
                m,
                max_elements,
                max_layer,
                ef_construction,
                DistDot {},
            )),
        };
        Ok(HnswIndex { graph })
    }

    /// Insert one vector under its slot number. hnsw_rs uses interior mutability
    /// (`&self`); the collection serializes inserts under its write lock.
    pub(crate) fn insert(&self, vector: &[f32], slot: usize) {
        match &self.graph {
            Graph::Cosine(h) => h.insert((vector, slot)),
            Graph::L2(h) => h.insert((vector, slot)),
            Graph::Dot(h) => h.insert((vector, slot)),
        }
    }

    /// Insert a batch in parallel via hnsw_rs's rayon-backed `parallel_insert`
    /// (scoped, join-before-return — CORE-052). Native-only, like the whole
    /// module, so no wasm32 concern.
    pub(crate) fn insert_batch(&self, items: &[(usize, Vec<f32>)]) {
        let batch: Vec<(&Vec<f32>, usize)> = items.iter().map(|(slot, v)| (v, *slot)).collect();
        match &self.graph {
            Graph::Cosine(h) => h.parallel_insert(&batch),
            Graph::L2(h) => h.parallel_insert(&batch),
            Graph::Dot(h) => h.parallel_insert(&batch),
        }
    }

    /// k-NN query. Returns `(slot, raw_distance)` pairs straight from hnsw_rs —
    /// no tombstone filtering, no score transformation (CORE-035 ordering and
    /// `Hit` mapping happen at the query layer). Rejects out-of-range
    /// `ef_search` with `InvalidArgument` (CORE-031).
    pub(crate) fn search(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<(usize, f32)>> {
        if !EF_SEARCH_BOUNDS.contains(&ef_search) {
            return Err(VecLiteError::InvalidArgument(format!(
                "hnsw ef_search must be in {}..={}, got {ef_search}",
                EF_SEARCH_BOUNDS.start(),
                EF_SEARCH_BOUNDS.end()
            )));
        }
        let neighbours: Vec<Neighbour> = match &self.graph {
            Graph::Cosine(h) => h.search(query, k, ef_search),
            Graph::L2(h) => h.search(query, k, ef_search),
            Graph::Dot(h) => h.search(query, k, ef_search),
        };
        Ok(neighbours
            .into_iter()
            .map(|n| (n.d_id, n.distance))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nearest_slot(index: &HnswIndex, query: &[f32]) -> usize {
        let hits = index.search(query, 1, 32).unwrap_or_else(|e| panic!("{e}"));
        let Some(&(slot, _)) = hits.first() else {
            panic!("expected at least one neighbour")
        };
        slot
    }

    #[test]
    fn euclidean_finds_nearest() {
        let index =
            HnswIndex::new(Metric::Euclidean, 3, 16, 200, 100).unwrap_or_else(|e| panic!("{e}"));
        index.insert(&[0.0, 0.0, 0.0], 0);
        index.insert(&[10.0, 10.0, 10.0], 1);
        index.insert(&[-5.0, -5.0, -5.0], 2);
        assert_eq!(nearest_slot(&index, &[9.5, 9.5, 9.5]), 1);
        assert_eq!(nearest_slot(&index, &[0.1, 0.0, -0.1]), 0);
    }

    #[test]
    fn cosine_finds_nearest_direction() {
        // Unit vectors (as the collection normalizes at ingest under Cosine).
        let index =
            HnswIndex::new(Metric::Cosine, 2, 16, 200, 100).unwrap_or_else(|e| panic!("{e}"));
        index.insert(&[1.0, 0.0], 0);
        index.insert(&[0.0, 1.0], 1);
        index.insert(&[-1.0, 0.0], 2);
        assert_eq!(nearest_slot(&index, &[0.9, 0.1]), 0);
        assert_eq!(nearest_slot(&index, &[0.1, 0.9]), 1);
    }

    #[test]
    fn dot_product_bounded_inputs() {
        // DistDot asserts dot <= 1, so keep inputs unit-length here.
        let index =
            HnswIndex::new(Metric::DotProduct, 2, 16, 200, 100).unwrap_or_else(|e| panic!("{e}"));
        index.insert(&[1.0, 0.0], 0);
        index.insert(&[0.0, 1.0], 1);
        assert_eq!(nearest_slot(&index, &[1.0, 0.0]), 0);
    }

    #[test]
    fn batch_insert_is_order_independent_for_recall() {
        let index =
            HnswIndex::new(Metric::Euclidean, 3, 16, 200, 100).unwrap_or_else(|e| panic!("{e}"));
        index.insert_batch(&[
            (0, vec![0.0, 0.0, 0.0]),
            (1, vec![10.0, 10.0, 10.0]),
            (2, vec![-5.0, -5.0, -5.0]),
        ]);
        assert_eq!(nearest_slot(&index, &[9.0, 9.0, 9.0]), 1);
    }

    #[test]
    fn parameter_bounds_are_enforced() {
        let Err(_) = HnswIndex::new(Metric::Euclidean, 3, 3, 200, 100) else {
            panic!("m below 4 must be rejected")
        };
        let Err(_) = HnswIndex::new(Metric::Euclidean, 3, 65, 200, 100) else {
            panic!("m above 64 must be rejected")
        };
        let Err(_) = HnswIndex::new(Metric::Euclidean, 3, 16, 4, 100) else {
            panic!("ef_construction below 8 must be rejected")
        };
        let index =
            HnswIndex::new(Metric::Euclidean, 3, 16, 200, 100).unwrap_or_else(|e| panic!("{e}"));
        index.insert(&[0.0, 0.0, 0.0], 0);
        let Err(_) = index.search(&[0.0, 0.0, 0.0], 1, 0) else {
            panic!("ef_search below 1 must be rejected")
        };
        let Err(_) = index.search(&[0.0, 0.0, 0.0], 1, 4097) else {
            panic!("ef_search above 4096 must be rejected")
        };
    }

    #[test]
    fn insert_batch_dispatches_every_metric() {
        // The parallel batch path has one arm per metric graph; exercise all.
        for metric in [Metric::Cosine, Metric::Euclidean, Metric::DotProduct] {
            let index = HnswIndex::new(metric, 3, 16, 200, 100).unwrap_or_else(|e| panic!("{e}"));
            index.insert_batch(&[(0, vec![1.0, 0.0, 0.0]), (1, vec![0.0, 1.0, 0.0])]);
            let hits = index
                .search(&[1.0, 0.0, 0.0], 1, 32)
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(hits.first().map(|(s, _)| *s), Some(0), "metric {metric:?}");
        }
    }
}
