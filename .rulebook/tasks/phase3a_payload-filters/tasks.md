## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-006 in full; DAG T3.1–T3.3; Vectorizer Qdrant filter model
- [x] 1.2 Payload storage: PAYLOAD segments (phase2a), 16 MiB limit + `_`-prefix reservation enforced in prepare() (FLT-001..003)
- [x] 1.3 Filter data model: Filter/Condition/MatchValue/Range + JSON shape (src/filter/mod.rs, FLT-010)
- [x] 1.4 Condition semantics: Eq/In/Range/Exists incl. missing-key/null rules (FLT-011); geo/nested-path rejection (FLT-012)
- [x] 1.5 Payload indexes keyword/int/float over roaring bitmaps, declared at creation, rebuilt from payloads on open (src/filter/index.rs, FLT-020/021)
- [x] 1.6 Execution: index-candidate pre-filter vs full-scan post-filter, always applying the full filter — exact and identical either way (FLT-030 correctness/031)

## 2. Testing
- [x] 2.1 Semantics corpus: combination + portable-JSON cases (tests/filters.rs)
- [x] 2.2 Index/scan equivalence (filters::index_and_scan_agree, FLT-022)
- [x] 2.3 Pre-filter correctness: selective filter equals brute-force filtered top-k (filters::prefilter_matches_bruteforce_topk)
- [x] 2.4 Reserved-key and unsupported-feature rejection (filters::reserved_underscore_keys_rejected, unsupported_features_rejected_at_query)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (CHANGELOG, README, SPEC-006 status)
- [x] 3.2 Write tests covering the new behavior (17 filter unit tests + 8 integration tests)
- [x] 3.3 Run tests and confirm they pass (all suites green x3; clippy clean; wasm32 builds)

<!-- Runtime create_payload_index (PIDX_DECLARE backfill, FLT-020 late creation) and
     the HNSW over-fetch post-filter strategy (FLT-030 acceleration) are tracked in
     phase3e_filter-runtime-index-hnsw-prefilter — both are optimizations over the
     exact, correct core delivered here (results are identical, FLT-022/031).
     Filtered scroll (FLT-032) lands with scroll in phase3d. -->
