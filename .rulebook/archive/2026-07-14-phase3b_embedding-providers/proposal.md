# Proposal: phase3b_embedding-providers

## Why
DAG T3.5 + T3.6: the offline 5-line quickstart depends on the pure-Rust sparse providers (bm25 default, tfidf, bow, char_ngram) and on vocabulary state that persists inside the file so a .veclite searches identically on any machine (FR-41, FR-42, FR-44).

## What Changes
- Embedder trait (sync, object-safe) with export_state/import_state (EMB-010)
- Providers extracted from Vectorizer: bm25 (k1=1.5, b=0.75), tfidf, bow, char_ngram; svd behind feature (EMB provider matrix)
- register_embedder per-Database custom providers (EMB-011)
- Vocabulary lifecycle: incremental updates + VOCAB_UPDATE WAL journaling, coalesced per checkpoint (EMB-030)
- refit(): explicit full recompute + re-embed, never automatic (EMB-031/032)
- Fail-fast rules: UnsupportedProvider listing available, dimension conflicts at creation (EMB-021)

## Impact
- Affected specs: SPEC-005 §1–5
- Affected code: crates/veclite/src/embedding/, VOCAB segment integration
- Breaking change: NO
- User benefit: text search that works offline with zero dependencies and survives file reopen byte-identically
