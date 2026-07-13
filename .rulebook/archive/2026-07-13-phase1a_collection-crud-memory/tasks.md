## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-001 §3–4/§7, SPEC-004 §1/§2/§4; DAG T1.1, T1.5
- [x] 1.2 Point/Hit/SparseVector structs; id and collection-name validation (CORE-010/011)
- [x] 1.3 Collection registry over DashMap: create/get/delete/rename + AlreadyExists/CollectionNotFound semantics
- [x] 1.4 Vector CRUD with slot storage: upsert/upsert_batch/get/delete/delete_batch/len
- [x] 1.5 Ingest guards: DimensionMismatch, NaN/Inf rejection, Cosine normalization (CORE-012..014)
- [x] 1.6 VecLite::memory() constructor; Send + Sync + Clone handles (CORE-050)

## 2. Testing
- [x] 2.1 Property tests: arbitrary op sequences vs model HashMap — state equivalence
- [x] 2.2 Concurrency smoke: parallel readers + serialized writers on one collection

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation
- [x] 3.2 Write tests covering the new behavior
- [x] 3.3 Run tests and confirm they pass
