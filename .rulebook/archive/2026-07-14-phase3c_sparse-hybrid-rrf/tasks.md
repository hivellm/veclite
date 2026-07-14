## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-007 in full; DAG T3.4, T3.7; Vectorizer hybrid model
- [x] 1.2 SparseVector::validate (HYB-001) wired into upsert; auto-embed rejects explicit sparse (HYB-002)
- [x] 1.3 Sparse dot-product search (Collection::search_sparse, HYB-003); SPARSE segment persistence + WAL rebuild moved to phase3g (HYB-030)
- [x] 1.4 hybrid_query() builder: dense/sparse lanes, alpha, rrf_k, limit, filter (HYB-010/011); .text() lane moved to phase3g
- [x] 1.5 RRF fusion: limit_fetch = max(limit*4, 100) per lane; formula + tie-breaking (HYB-020/021)
- [x] 1.6 Degenerate single-lane equivalence to plain search (HYB-010)
- [x] 1.7 Tombstone-aware postings iteration; SPARSE vacuum rewrite moved to phase3g (HYB-031)

## 2. Testing
- [x] 2.1 RRF fused ordering + determinism (tests/hybrid.rs); server conformance corpus moved to phase3g (HYB-022)
- [x] 2.2 Determinism: repeated identical hybrid queries return identical orderings (hybrid::rrf_fusion_is_deterministic)
- [x] 2.3 Crash-recovery of the sparse index moved to phase3g (needs SPARSE persistence)
- [x] 2.4 Filtered hybrid equals the filtered reference (hybrid::filter_applies_to_both_lanes)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (CHANGELOG, README, SPEC-007 status)
- [x] 3.2 Write tests covering the new behavior (7 hybrid integration tests + sparse validation)
- [x] 3.3 Run tests and confirm they pass (all suites green; clippy clean; wasm32 builds)

<!-- SPARSE segment persistence + reopen/vacuum (HYB-030/031), the auto-embed
     .text() lane (HYB-011), and the server conformance corpus (HYB-022) are
     tracked in phase3g_sparse-persistence-conformance — persistence and
     cross-repo fixtures over the deterministic core delivered here. -->
