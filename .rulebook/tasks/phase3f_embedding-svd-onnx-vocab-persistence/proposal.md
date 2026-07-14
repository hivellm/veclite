# Proposal: phase3f_embedding-svd-onnx-vocab-persistence

## Why

phase3b delivered the Embedder trait, the four default sparse providers
(bm25/tfidf/bow/char_ngram), the auto-embed collection API
(upsert_text/search_text/refit), the chunker, and reopen-deterministic text
search (the vocabulary rebuilds from the stored `_text`, like the HNSW graph).
Several SPEC-005 items remain — each an optimization, a feature-gated provider,
or a piece needing server fixtures — on top of a correct core:

1. Incremental vocabulary + VOCAB persistence (EMB-030): upsert_text currently
   marks the vocabulary stale and the next search does a full refit (fit +
   re-embed every document). That is exact and reopen-deterministic but O(n)
   per search after a batch, and re-embedding tombstones the old slots. The
   server's incremental IDF (approximate) plus a persisted VOCAB segment /
   VOCAB_UPDATE WAL avoids the full recompute.
2. register_embedder (EMB-011): per-Database custom providers; a registered name
   unknown on reopen must fail with UnsupportedProvider saying so.
3. svd provider behind the `svd` feature (TF-IDF + truncated SVD), and the
   `onnx` feature (fastembed) as separate artifacts (EMB-040/041, phase5).
4. Server parity corpus (EMB acceptance 1): a shared (text, state, score) corpus
   proving bm25/tfidf/bow/char_ngram match the server within 1e-5.

## What Changes

- Incremental add_document on the trainable providers + a VOCAB segment
  (export_state/import_state) written at checkpoint and imported on open;
  VOCAB_UPDATE WAL op (8) wired in replay.
- Database::register_embedder(name, Box<dyn Embedder>).
- svd (feature `svd`, dep ndarray) and the onnx/fastembed distribution.
- A conformance corpus generated once from the server and enforced in CI.

## Impact

- Affected specs: SPEC-005 §3 (EMB-011), §5 (EMB-030), §2/§6 (svd/onnx),
  acceptance 1
- Affected code: crates/veclite/src/embedding/*, persist (VOCAB segment + WAL),
  database.rs (register_embedder)
- Breaking change: NO (additive; the exact refit path stays the baseline)
- User benefit: fast incremental text ingestion, custom providers, dense neural
  embeddings, and a pinned parity guarantee
