# Proposal: phase3g_sparse-persistence-conformance

## Why

phase3c delivered the deterministic hybrid core: SparseVector validation, sparse
dot-product search, the hybrid_query() builder, RRF fusion (server-parity
formula and tie-breaking), single-lane degeneration, and filtered hybrid — all
in memory and tested. The pieces that remain are persistence, the auto-embed
sparse lane, and the cross-repo parity proof:

1. SPARSE segment persistence (HYB-030): the BYO sparse lane is stored in memory
   (and recovered via WAL replay) but not sealed, so it is lost after a
   checkpoint+reopen. Seal/load must carry a SPARSE segment; vacuum must rewrite
   it dropping tombstoned slots (HYB-031).
2. Auto-embed .text() lane (HYB-011): on a bm25 auto-embed collection, .text(q)
   should embed the query for the dense lane AND derive its sparse weights from
   the provider vocabulary, filling both lanes from one string.
3. Server conformance corpus (HYB-022, gate G3): a shared (corpus, query,
   fused-ranking) fixture proving VecLite's RRF ordering matches the server, and
   a crash-recovery test (sparse index after kill-9 + replay == rebuilt).

## What Changes

- SPARSE segment type wired into seal::seal / seal::load and the pager; sparse
  survives checkpoint+reopen; vacuum/compact rebuild it.
- HybridQuery::text(&str) on auto-embed collections → dense embed + sparse term
  weights from the provider.
- A committed conformance corpus + a parity test (shared with the server repo).

## Impact

- Affected specs: SPEC-007 HYB-011/030/031, acceptance 1/4
- Affected code: persist/seal.rs, storage SPARSE segment, collection.rs
  (compact/hybrid text lane), a conformance test
- Breaking change: NO (additive; the in-memory hybrid results are unchanged)
- User benefit: hybrid survives reopen, one-string hybrid on auto-embed
  collections, and a pinned parity guarantee
