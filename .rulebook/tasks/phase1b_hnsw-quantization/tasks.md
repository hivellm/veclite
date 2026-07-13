## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-001 §5–6; DAG T1.2, T1.3; Vectorizer source db/optimized_hnsw.rs
- [ ] 1.2 Pin hnsw_rs =0.3.x; extract sync CPU HNSW wrapper (strip async/GPU/shard branches, note provenance)
- [ ] 1.3 Parameter bounds validation per CORE-031; per-collection m/ef_construction, default ef_search
- [ ] 1.4 Soft-delete tombstone set + over-fetch on search; upsert-existing = soft-delete + insert (CORE-032/033)
- [ ] 1.5 reindex(): rebuild graph from live vectors, purge tombstones
- [ ] 1.6 Vendor quantization from Vectorizer crates/vectorizer-core/src/quantization (scalar/binary; product behind pq) with provenance headers, byte-identical encodings (ADR-0001, CORE-001)
- [ ] 1.7 Vendor SIMD distance kernels from crates/vectorizer-core/src/simd behind the simd feature, scalar fallback always available (ADR-0001)
- [ ] 1.8 SQ-8 encode on ingest + quantized-domain scoring; None/Binary options (CORE-040..043)
- [ ] 1.9 rayon scoped-thread batch insert, cfg-gated off for wasm32 (CORE-052)

## 2. Testing
- [ ] 2.1 Recall harness vs brute force: top-10 recall >= 0.95 at defaults
- [ ] 2.2 SQ-8 recall >= 0.99 vs unquantized on the standard corpus
- [ ] 2.3 Tombstone correctness: delete half, search returns only live vectors at full limit

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
