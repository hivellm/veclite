# phase3c hybrid: RRF fusion is deterministic if you rank by (fused desc, dense rank, bytewise id) and always project from live data
**Source**: manual
**Date**: 2026-07-14
**Related Task**: phase3c_sparse-hybrid-rrf
**Tags**: phase3c, hybrid, rrf, sparse, fusion, veclite
phase3c delivered the hybrid dense+sparse RRF core (SPEC-007). Design notes:

1. RRF determinism (HYB-021) hinges entirely on the tie-break chain: order fused candidates by (fused_score DESC, dense_rank ASC with absent=usize::MAX, then bytewise id). Using the dense rank as the second key (not the sparse rank) matches the server. Ranks are 1-based within each lane; a doc absent from a lane contributes 0 to the fused score and MAX to the dense-rank tiebreak. limit_fetch per lane = max(limit*4, 100).

2. Single-lane degeneration (HYB-010) must return the lane's OWN scores, not RRF scores: if only dense is provided, execute_hybrid just calls execute_query (so hybrid==search exactly); only sparse → search_sparse. Tested by asserting the full Hit vectors are equal, not just ids.

3. Sparse scoring is a linear merge dot product over the two sorted index arrays (SparseVector::dot). search_sparse and the sparse lane share one sparse_ranked(query, limit, filter) helper: it brute-forces live slots, skips filter-failing payloads, keeps only non-zero dot scores, sorts by (score desc, id), truncates. So filtered hybrid applies the filter to both lanes for free (dense via execute_query's filter param, sparse via sparse_ranked's filter).

4. Fusion projects the final Hits from the LIVE data under one read lock (id_to_slot lookup), not from the lane Hits — this keeps payload/vector projection consistent and avoids trusting stale lane snapshots. The dense lane runs with with_payload=false (we only need its order), so no wasted cloning.

5. Scope split: the BYO sparse lane is in-memory only (recovered by WAL replay, but NOT sealed) — sparse is lost after checkpoint+reopen until SPARSE segment persistence lands (phase3g). The auto-embed .text() lane (embed for dense + provider-derived sparse weights) and the server conformance corpus also → phase3g. The in-memory RRF core is exact and fully tested; persistence and cross-repo parity are the follow-up.

Validation (HYB-001) lives on SparseVector::validate and is wired into prepare_inner (always), plus HYB-002 (auto-embed rejects explicit sparse) gated to the public path (!allow_reserved) so WAL replay of a BYO collection is unaffected.