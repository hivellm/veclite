## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-007 in full; DAG T3.4, T3.7; Vectorizer db/hybrid_search.rs
- [ ] 1.2 SparseVector validation (HYB-001); per-collection sparse mode auto vs BYO with mixing rejection (HYB-002)
- [ ] 1.3 Inverted index + SPARSE segment persistence; WAL delta application on upsert/delete (HYB-003, HYB-030)
- [ ] 1.4 hybrid_query() builder: dense/sparse/text lanes, alpha, rrf_k, limit, filter (HYB-010/011)
- [ ] 1.5 RRF fusion: limit_fetch = max(limit*4, 100) per lane; formula and tie-breaking per HYB-020/021
- [ ] 1.6 Degenerate single-lane equivalence to plain search (HYB-010)
- [ ] 1.7 Tombstone-aware postings iteration; vacuum rewrite (HYB-031)

## 2. Testing
- [ ] 2.1 RRF conformance vs server: identical fused rankings on the shared corpus (HYB-022)
- [ ] 2.2 Determinism: repeated identical hybrid queries return identical orderings
- [ ] 2.3 Crash-recovery: sparse index after kill-9 + replay equals rebuilt-from-scratch
- [ ] 2.4 Filtered hybrid equals brute-force reference on the test corpus

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
