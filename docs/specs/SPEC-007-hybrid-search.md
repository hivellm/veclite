# SPEC-007 — Sparse Vectors & Hybrid Search

| | |
|---|---|
| **Status** | Implemented (phase3c): SparseVector validation, sparse dot-product search, the hybrid_query() builder, deterministic RRF fusion (formula + tie-breaking, HYB-020/021), single-lane degeneration, and filtered hybrid. SPARSE segment persistence (HYB-030/031), the auto-embed `.text()` lane (HYB-011), and the server conformance corpus (HYB-022) are tracked in `phase3g_sparse-persistence-conformance`. |
| **Phase / tasks** | Phase 3 · T3.4, T3.7 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-34 |
| **Planning source** | [05-embeddings.md §hybrid](../vectorizer-lite/05-embeddings.md), server sources `db/hybrid_search.rs`, `models/sparse_vector.rs` |

Requirement IDs `HYB-xxx`.

## 1. Sparse vectors

```rust
pub struct SparseVector { pub indices: Vec<u32>, pub values: Vec<f32> }
```

- **HYB-001** `indices` MUST be strictly increasing (sorted, unique); `values.len() == indices.len()`; values finite. Violations → `InvalidArgument`.
- **HYB-002** Two sources of sparse data per collection: (a) **auto-embed BM25 collections** maintain the sparse lane automatically from `_text`; (b) **BYO users** supply `Point.sparse` explicitly. A collection uses one mode; supplying an explicit sparse vector to an auto-embed collection → `InvalidArgument`.
- **HYB-003** Sparse scoring is dot product over the shared term space (BM25 weights on the auto path — server parity). The inverted index (`term_id → postings (slot, weight)`) persists as SPARSE segments (STG §3.1).

## 2. Hybrid query API

```rust
let hits = docs.hybrid_query()
    .dense(&query_vec)              // optional if sparse given
    .sparse(&sparse_query)          // optional if dense given; auto-embed: .text("query")
    .text("query text")             // auto-embed collections: fills BOTH lanes
    .alpha(0.5)                     // dense weight ∈ [0,1]; default 0.5
    .limit(10)
    .filter(filter)                 // SPEC-006 filter applies to both lanes
    .rrf_k(60)                      // RRF constant; default 60 (server parity)
    .run()?;
```

- **HYB-010** At least one lane MUST be provided (`InvalidArgument` otherwise). Single-lane hybrid degenerates to plain dense or sparse search (same scores as the dedicated APIs).
- **HYB-011** `.text(q)` is valid only on auto-embed collections: it embeds the query for the dense lane and tokenizes/weights it for the sparse lane using the collection's provider state.

## 3. Fusion (Reciprocal Rank Fusion)

- **HYB-020** Each lane retrieves its own top-`limit_fetch` candidates (`limit_fetch = max(limit × 4, 100)`, internal, tunable). Fusion score:

  `score(d) = alpha · 1/(rrf_k + rank_dense(d)) + (1 − alpha) · 1/(rrf_k + rank_sparse(d))`

  where a document absent from a lane contributes 0 for that lane. Ranks are 1-based within each lane.
- **HYB-021** Results ordered by fused score descending; ties broken by dense rank, then id (bytewise) — fully deterministic.
- **HYB-022** Semantics (formula, defaults `alpha = 0.5`, `rrf_k = 60`, tie-breaking) MUST match the server's hybrid search; the conformance corpus pins fused rankings, not just sets (gate G3).
- **HYB-023** `Hit.score` for hybrid results is the fused score. Per-lane scores are exposed only under the `explain` feature.

## 4. Persistence & recovery

- **HYB-030** The sparse inverted index rebuilds from SPARSE segments at open and applies WAL deltas (upserts/deletes journal their sparse component inside `UPSERT_BATCH`/`DELETE_BATCH` bodies — no separate op).
- **HYB-031** Tombstoned slots are excluded from postings iteration at query time; vacuum rewrites SPARSE segments dropping them.

## 5. Acceptance criteria (gate G3)

1. RRF conformance vs server: identical fused rankings on the shared corpus (HYB-022).
2. Degenerate-lane equivalence: hybrid with only dense ≡ `search`; only sparse ≡ sparse-only search (HYB-010).
3. Determinism test: repeated identical hybrid queries return identical orderings (HYB-021).
4. Crash-recovery test: sparse index state after kill-9 + replay ≡ rebuilt-from-scratch state.
5. Filtered hybrid: filter applied to both lanes; results equal brute-force reference on the test corpus.
