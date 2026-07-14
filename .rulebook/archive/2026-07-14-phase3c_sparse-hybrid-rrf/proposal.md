# Proposal: phase3c_sparse-hybrid-rrf

## Why
DAG T3.4 + T3.7: hybrid dense+sparse retrieval with RRF fusion is a core differentiator vs peer embedded stores and must rank identically to the Vectorizer server (FR-34).

## What Changes
- SparseVector validation (strictly increasing indices, finite values) — HYB-001
- Two sparse sources per collection: auto (bm25 from _text) or explicit BYO; mixing rejected (HYB-002)
- SPARSE postings segments (term_id → slot, weight) + in-memory inverted index with WAL delta application (HYB-003, HYB-030)
- hybrid_query() builder: dense/sparse/text lanes, alpha (default 0.5), rrf_k (default 60), filter on both lanes (HYB-010/011)
- RRF fusion with deterministic tie-breaking (dense rank then bytewise id) — HYB-020..023
- Tombstone exclusion in postings iteration; vacuum rewrites SPARSE segments (HYB-031)

## Impact
- Affected specs: SPEC-007 (all)
- Affected code: crates/veclite/src/hybrid/, storage SPARSE integration
- Breaking change: NO
- User benefit: keyword+semantic fused ranking identical to the server, fully deterministic
