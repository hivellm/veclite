## 1. Implementation
- [ ] 1.1 Incremental add_document on trainable providers (approximate IDF) so upsert_text is O(doc), not a full refit (EMB-030; was phase3b lifecycle)
- [ ] 1.2 VOCAB segment persistence: export_state at checkpoint, import_state on open; wire VOCAB_UPDATE WAL op 8 in apply_wal_entry (EMB-030)
- [ ] 1.3 Database::register_embedder(name, Box<dyn Embedder>) per-instance; unknown-on-reopen fails with UnsupportedProvider (EMB-011)
- [ ] 1.4 svd provider behind the `svd` feature (ndarray) (EMB matrix)
- [ ] 1.5 onnx/fastembed feature + separate distribution artifacts (EMB-040/041; phase5 overlap)

## 2. Testing
- [ ] 2.1 Server parity corpus: bm25/tfidf/bow/char_ngram scores within 1e-5 given identical state (EMB acceptance 1)
- [ ] 2.2 Incremental-then-refit equals from-scratch rebuild; VOCAB persistence reopen determinism without re-embedding
- [ ] 2.3 register_embedder + unknown-on-reopen error

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
