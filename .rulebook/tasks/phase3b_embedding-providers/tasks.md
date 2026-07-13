## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-005 §1–5; DAG T3.5, T3.6; Vectorizer embedding/providers sources
- [ ] 1.2 Embedder trait + provider registry with per-Database register_embedder (EMB-010/011)
- [ ] 1.3 Extract bm25 provider (k1=1.5, b=0.75 server parity) with versioned internal state encoding
- [ ] 1.4 Extract tfidf, bow, char_ngram providers; svd behind the svd feature
- [ ] 1.5 Fail-fast creation rules: unknown provider, dimension conflict, text-op-on-BYO (EMB-021)
- [ ] 1.6 Vocabulary persistence: VOCAB segments + VOCAB_UPDATE WAL entries coalesced per checkpoint (EMB-030)
- [ ] 1.7 refit(): recompute vocabulary from stored _text, re-embed atomically per batch (EMB-031/032)

## 2. Testing
- [ ] 2.1 Provider score parity vs server: identical scores within 1e-5 given identical state
- [ ] 2.2 Reopen determinism: build, close, reopen — search_text results identical (EMB-020)
- [ ] 2.3 Fail-fast matrix unit tests incl. no-silent-fallback assertions
- [ ] 2.4 refit equals from-scratch rebuild on the same corpus

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
