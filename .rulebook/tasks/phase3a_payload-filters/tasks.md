## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-006 in full; DAG T3.1–T3.3; Vectorizer sources models/qdrant/filter.rs + filter_processor.rs
- [ ] 1.2 Payload storage: PAYLOAD segments, 16 MiB limit, _-prefix reservation enforcement (FLT-001..003)
- [ ] 1.3 Filter data model: Filter/Condition types + JSON shape matching the server (FLT-010)
- [ ] 1.4 Condition semantics: Eq/In/Range/Exists incl. missing-key and null rules (FLT-011); geo/nested-path rejection (FLT-012)
- [ ] 1.5 Payload indexes keyword/int/float over roaring bitmaps; late creation with backfill scan (FLT-020/021)
- [ ] 1.6 Execution strategies: selectivity estimate, pre-filter bitmap vs post-filter over-fetch (FLT-030)
- [ ] 1.7 Filtered scroll + query builder integration (FLT-032)

## 2. Testing
- [ ] 2.1 Semantics corpus table (payloads, filter doc, expected ids) — shared with server repo (gate G3 input)
- [ ] 2.2 Index/scan equivalence property test (FLT-022)
- [ ] 2.3 Pre-filter correctness: selective filters (<1 percent) equal brute-force filtered top-k
- [ ] 2.4 Reserved-key and unsupported-feature rejection tests

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
