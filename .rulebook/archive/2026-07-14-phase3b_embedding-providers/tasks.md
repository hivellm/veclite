## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-005 §1–5; DAG T3.5, T3.6; Vectorizer embedding/providers sources
- [x] 1.2 Embedder trait + provider factory build_provider (src/embedding/mod.rs, EMB-010; register_embedder → phase3f)
- [x] 1.3 bm25 provider (k1=1.5, b=0.75 server parity) with JSON export/import state (src/embedding/bm25.rs)
- [x] 1.4 tfidf, bow, char_ngram providers vendored (src/embedding/{tfidf,bow,char_ngram}.rs); svd → phase3f
- [x] 1.5 Fail-fast creation rules: unknown provider (before journaling), text-op-on-BYO (EMB-021)
- [x] 1.6 Vocabulary lifecycle: full-refit model — `_text` stored (EMB-022), rebuilt on open (reopen-deterministic, EMB-020); incremental + VOCAB persistence → phase3f
- [x] 1.7 refit(): recompute vocabulary from stored `_text` and re-embed (EMB-031/032)
- [x] 1.8 Chunker (src/chunk.rs): UTF-8-safe, word/sentence boundaries, overlap (EMB-050/051)

## 2. Testing
- [x] 2.1 Provider unit tests (bm25 idf/tf formula, tfidf/bow/char_ngram fit+embed+round-trip); server parity corpus → phase3f
- [x] 2.2 Reopen determinism: search_text identical after close/reopen (auto_embed::reopen_preserves_search_text_results, EMB-020)
- [x] 2.3 Fail-fast matrix: unknown provider, text-op-on-BYO, reserved key (tests/auto_embed.rs)
- [x] 2.4 refit keeps search working; chunker UTF-8 fuzz (no panics, no split code points)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (CHANGELOG, README, SPEC-005 status)
- [x] 3.2 Write tests covering the new behavior (13 provider/chunker unit tests + 7 auto-embed integration tests)
- [x] 3.3 Run tests and confirm they pass (all suites green; clippy clean; wasm32 builds)

<!-- Incremental vocabulary + VOCAB persistence (EMB-030), register_embedder
     (EMB-011), the svd/onnx providers, and the server parity corpus are tracked
     in phase3f_embedding-svd-onnx-vocab-persistence — optimizations, feature-gated
     providers, and server-fixture work over the correct core delivered here. -->
