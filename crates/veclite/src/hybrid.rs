//! Hybrid dense+sparse search with reciprocal rank fusion (SPEC-007 §2–3).
//!
//! [`HybridQuery`] is a plain builder — it holds no lock and touches no index
//! until [`run`](HybridQuery::run). Provide the dense lane, the sparse lane, or
//! both; a single lane degenerates to that lane's plain search (HYB-010), two
//! lanes fuse with RRF (`alpha` default 0.5, `rrf_k` default 60 — HYB-020..022).

use crate::collection::Collection;
use crate::error::Result;
use crate::filter::Filter;
use crate::point::{Hit, SparseVector};

/// Default RRF fusion weight for the dense lane (SPEC-007 HYB-020).
pub const DEFAULT_ALPHA: f32 = 0.5;
/// Default RRF constant (SPEC-007, server parity).
pub const DEFAULT_RRF_K: f32 = 60.0;
/// Default result limit (SPEC-004 §5).
const DEFAULT_LIMIT: usize = 10;

/// Fluent hybrid query over a [`Collection`] (SPEC-007 §2).
pub struct HybridQuery<'a> {
    collection: &'a Collection,
    dense: Option<&'a [f32]>,
    sparse: Option<&'a SparseVector>,
    /// One-string lane source for auto-embed collections (HYB-011); when set it
    /// supplies BOTH lanes at `run`, overriding explicit `dense`/`sparse`.
    text: Option<&'a str>,
    alpha: f32,
    rrf_k: f32,
    limit: usize,
    with_payload: bool,
    with_vector: bool,
    filter: Option<Filter>,
}

impl<'a> HybridQuery<'a> {
    pub(crate) fn new(collection: &'a Collection) -> Self {
        HybridQuery {
            collection,
            dense: None,
            sparse: None,
            text: None,
            alpha: DEFAULT_ALPHA,
            rrf_k: DEFAULT_RRF_K,
            limit: DEFAULT_LIMIT,
            with_payload: true,
            with_vector: false,
            filter: None,
        }
    }

    /// Fill **both** lanes from one query string (SPEC-007 HYB-011): valid only
    /// on auto-embed collections, where the dense lane is the provider
    /// embedding and the sparse lane its non-zero terms. Overrides any explicit
    /// [`dense`](Self::dense)/[`sparse`](Self::sparse). Fails at `run` with
    /// `InvalidArgument` on a BYO collection.
    #[must_use]
    pub fn text(mut self, query: &'a str) -> Self {
        self.text = Some(query);
        self
    }

    /// Provide the dense lane query vector.
    #[must_use]
    pub fn dense(mut self, vector: &'a [f32]) -> Self {
        self.dense = Some(vector);
        self
    }

    /// Provide the sparse lane query vector.
    #[must_use]
    pub fn sparse(mut self, sparse: &'a SparseVector) -> Self {
        self.sparse = Some(sparse);
        self
    }

    /// Dense-lane fusion weight in `[0, 1]` (default 0.5); the sparse lane gets
    /// `1 - alpha`. Clamped to `[0, 1]` at `run`.
    #[must_use]
    pub fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }

    /// RRF constant (default 60).
    #[must_use]
    pub fn rrf_k(mut self, rrf_k: f32) -> Self {
        self.rrf_k = rrf_k;
        self
    }

    /// Maximum number of fused hits (default 10).
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Include each hit's payload (default true).
    #[must_use]
    pub fn with_payload(mut self, with_payload: bool) -> Self {
        self.with_payload = with_payload;
        self
    }

    /// Include each hit's stored vector (default false).
    #[must_use]
    pub fn with_vector(mut self, with_vector: bool) -> Self {
        self.with_vector = with_vector;
        self
    }

    /// Apply a payload filter to **both** lanes (SPEC-007 HYB-011 / SPEC-006).
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Execute the hybrid query. Fails with `InvalidArgument` when no lane is
    /// provided (HYB-010).
    pub fn run(self) -> Result<Vec<Hit>> {
        // `.text(q)` resolves both lanes from the provider (HYB-011); the owned
        // dense/sparse live for this call and are passed by reference.
        if let Some(query) = self.text {
            let (dense, sparse) = self.collection.embed_for_hybrid(query)?;
            // No term of the query is in the vocabulary, so both lanes are
            // empty: "nothing matched", not a caller error. Returning here
            // keeps the text entry points consistent with `search_text` and
            // avoids surfacing the dense lane's cosine zero-vector guard, which
            // exists for callers who chose the vector and the metric.
            // Guarded on `limit > 0` so an invalid limit still reaches the
            // argument checks instead of being masked by an empty result.
            if self.limit > 0 && dense.iter().all(|v| *v == 0.0) && sparse.is_none() {
                return Ok(Vec::new());
            }
            return self.collection.execute_hybrid(
                Some(&dense),
                sparse.as_ref(),
                self.alpha.clamp(0.0, 1.0),
                self.rrf_k,
                self.limit,
                self.with_payload,
                self.with_vector,
                self.filter.as_ref(),
            );
        }
        self.collection.execute_hybrid(
            self.dense,
            self.sparse,
            self.alpha.clamp(0.0, 1.0),
            self.rrf_k,
            self.limit,
            self.with_payload,
            self.with_vector,
            self.filter.as_ref(),
        )
    }
}
