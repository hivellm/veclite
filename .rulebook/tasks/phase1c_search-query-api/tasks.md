## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-004 §4–5, SPEC-001 §5 (CORE-035); DAG T1.4
- [ ] 1.2 Hit struct wiring: id, score, optional payload, optional vector
- [ ] 1.3 Collection::search(vector, limit) with ordering per metric (CORE-035)
- [ ] 1.4 QueryBuilder: limit / ef_search / with_payload / with_vector; lock-free until run() (API-030)
- [ ] 1.5 Input validation: limit=0 rejected, ef_search bounds (CORE-031), query dimension check
- [ ] 1.6 Filter builder slot declared (type only, evaluation lands in phase3a)

## 2. Testing
- [ ] 2.1 Ordering tests per metric (Cosine/DotProduct desc, Euclidean asc)
- [ ] 2.2 Builder option matrix tests incl. defaults with_payload=true / with_vector=false
- [ ] 2.3 Edge cases: limit > live count, empty collection, wrong query dimension

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
