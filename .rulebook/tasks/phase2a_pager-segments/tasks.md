## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-002 §1–5 in full; DAG T2.1, T2.2
- [ ] 1.2 Header struct + encode/decode + crc validation + min_reader_version gate (STG-010/011)
- [ ] 1.3 Vendor compression helpers (lz4/zstd wrappers) from Vectorizer crates/vectorizer-core/src/compression with provenance headers (ADR-0001)
- [ ] 1.4 Segment header codec (32 bytes) + crc32 body verification naming offset/type on failure (STG-021)
- [ ] 1.5 Segment bodies: CONFIG, TOMBSTONE (roaring), PAYLOAD, PIDX, SPARSE, HNSW, VOCAB, IDDIR (MessagePack per STG §3.1)
- [ ] 1.6 VECTORS fixed-stride body: f32/sq8/sq4/sq2/sq1/binary encodings, slot addressing, never compressed (STG-030/031)
- [ ] 1.7 IDDIR hash-bucketed directory with collision handling (STG-032)
- [ ] 1.8 TOC encode/decode with generation counter + deterministic load order (STG-040/041)
- [ ] 1.9 Commit protocol implementation: root-pointer swap sequence (STG-050)

## 2. Testing
- [ ] 2.1 Property round-trip tests for every segment type and the TOC
- [ ] 2.2 Header fuzz: truncated/bit-flipped headers fail with Corrupt, never panic
- [ ] 2.3 Commit-sequence test: simulated crash between each protocol step leaves previous TOC valid

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
