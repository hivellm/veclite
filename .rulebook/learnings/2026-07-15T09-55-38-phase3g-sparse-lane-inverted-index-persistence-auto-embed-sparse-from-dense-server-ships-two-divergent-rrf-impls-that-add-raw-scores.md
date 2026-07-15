# phase3g: sparse lane inverted-index persistence, auto-embed sparse-from-dense; server ships two divergent RRF impls that add raw scores
**Source**: manual
**Date**: 2026-07-15
**Related Task**: phase3g_sparse-persistence-conformance
**Tags**: phase3g, hybrid, sparse, rrf, persistence, conformance, veclite
phase3g (SPARSE persistence + .text() lane + RRF conformance):

1. In-memory sparse is FORWARD (data.sparses: Vec<Option<SparseVector{indices,values}>>), on-disk is INVERTED (SPARSE segment SparsePostings: term_id -> [(slot,weight)], codec already existed in body.rs from phase2b). seal converts forward->inverted via a BTreeMap<u32, Vec<(u64,f32)>> (ascending terms + slot-order postings = deterministic body for golden files). load converts inverted->forward: iterate terms ASCENDING and push term_id to each slot's indices → indices come out sorted for free (HYB-001 invariant, no re-sort).

2. LivePoint was (id, vec, payload) — adding sparse makes it a 4-tuple, rippling to ~8 sites (seal loops use `_`, load points.push, live_points, database install→Point, LoadedBase+install_base for the mmap tier, seal tests). WAL needed NOTHING: Point.sparse already serializes inside UPSERT_BATCH, so recovery = sealed SPARSE ∪ WAL replay automatically. Vacuum/compact drop tombstoned postings for FREE because seal builds the index from live_points only.

3. .text() on the borrow-based HybridQuery builder: can't hold owned embeddings in `&'a [f32]` fields. Solution: a `text: Option<&'a str>` field; run() branches — if text set, `embed_for_hybrid(q)` returns owned (Vec<f32>, Option<SparseVector>) as locals and passes Some(&dense)/sparse.as_ref() into execute_hybrid (owned data lives for the call). No lifetime gymnastics.

4. For .text() to exercise a sparse lane, auto-embed docs MUST store sparse (HYB-002a "maintain the sparse lane"). Chose sparse = non-zero components of the dense bm25 embedding (sparse_from_dense helper), set in upsert_text AND do_refit. Redundant for bm25 (dense==sparse source) but spec-literal and forward-compatible (onnx dense + bm25 sparse). Persisted by the SPARSE segment. Doesn't disturb phase3f: search_text is dense-only, tests unaffected.

5. SERVER RRF IS INCONSISTENT: db/hybrid_search.rs AND discovery/hybrid.rs both compute `rrf_score + score*alpha` (rank-RRF + raw-score hybrid) and DISAGREE with each other; neither is pure RRF. SPEC-007 HYB-020 defines VecLite's fusion as PURE rank-based RRF (alpha/(k+dr) + (1-alpha)/(k+sr)), which phase3c implemented. Decision: keep pure RRF (deterministic, corpus-independent, metric-agnostic), pin it with a formula-derived conformance corpus, and DOCUMENT the divergence in SPEC-007 status + the test. Do NOT chase byte-parity with an inconsistent server.

6. Conformance corpus construction trick: fixture gives dense-order (ALL ids) + sparse-subset-order; a real Euclidean collection reproduces them — doc at position r sits at distance r (dense ranking), sparse-subset docs get index-0 weights descending by sparse rank, others get no sparse lane. Every doc is dense-ranked in VecLite (fetch>=n), so the fixture MUST list all ids in `dense`.