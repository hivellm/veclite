//! k-NN query builder (SPEC-004 §5).
//!
//! A [`QueryBuilder`] is plain data — it holds no lock and touches no index
//! until [`run`](QueryBuilder::run) (API-030). `Collection::search` is the
//! one-call form; `Collection::query` returns this builder for per-query
//! overrides (limit, `ef_search`, payload/vector projection, and — from
//! phase3a — payload filters).

use crate::collection::Collection;
use crate::error::Result;
use crate::point::Hit;

/// Default result limit when the builder's `limit` is left untouched
/// (SPEC-004 §5).
pub(crate) const DEFAULT_LIMIT: usize = 10;

/// Opaque payload filter (SPEC-004 §5). The type is declared here so the
/// builder slot exists; its conditions and evaluation land in phase3a
/// (SPEC-006). It has no public constructor yet, so a query cannot carry one.
#[derive(Clone, Debug)]
pub struct Filter {
    _private: (),
}

/// Fluent k-NN query over a [`Collection`] (SPEC-004 §5).
///
/// Every setter consumes and returns the builder; nothing runs — and no lock
/// is taken — until [`run`](Self::run) (API-030).
pub struct QueryBuilder<'a> {
    collection: &'a Collection,
    vector: &'a [f32],
    limit: usize,
    ef_search: Option<usize>,
    with_payload: bool,
    with_vector: bool,
    filter: Option<Filter>,
}

impl<'a> QueryBuilder<'a> {
    /// Server-parity defaults: `limit = 10`, collection `ef_search`,
    /// `with_payload = true`, `with_vector = false` (SPEC-004 §3/§5).
    pub(crate) fn new(collection: &'a Collection, vector: &'a [f32]) -> Self {
        QueryBuilder {
            collection,
            vector,
            limit: DEFAULT_LIMIT,
            ef_search: None,
            with_payload: true,
            with_vector: false,
            filter: None,
        }
    }

    /// Maximum number of hits to return. `0` is rejected at `run` (API-031);
    /// a limit above the live count simply returns all live vectors.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Override the collection's default `ef_search` for this query
    /// (bounds enforced at `run`, CORE-031).
    #[must_use]
    pub fn ef_search(mut self, ef_search: usize) -> Self {
        self.ef_search = Some(ef_search);
        self
    }

    /// Include each hit's stored payload (default `true`).
    #[must_use]
    pub fn with_payload(mut self, with_payload: bool) -> Self {
        self.with_payload = with_payload;
        self
    }

    /// Include each hit's stored vector (default `false`).
    #[must_use]
    pub fn with_vector(mut self, with_vector: bool) -> Self {
        self.with_vector = with_vector;
        self
    }

    /// Attach a payload filter. Declared for API stability; evaluation lands in
    /// phase3a (SPEC-006). `Filter` has no public constructor yet.
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Execute the query. This is the only method that takes the read lock.
    pub fn run(self) -> Result<Vec<Hit>> {
        self.collection.execute_query(
            self.vector,
            self.limit,
            self.ef_search,
            self.with_payload,
            self.with_vector,
            self.filter.as_ref(),
        )
    }
}
