## 1. Implementation
- [ ] 1.1 cbindgen header generation + committed golden veclite.h; drift test (FFI-006)
- [ ] 1.2 cargo public-api snapshot committed + CI additive-only check (API-062, T4.1 freeze)
- [ ] 1.3 Remaining functions: upsert_batch/delete_batch, hybrid_search, search_batch (+vl_hits_batch), scroll (+vl_page), chunk, reindex/refit, payload_index_create, snapshot/vacuum/db_info
- [ ] 1.4 Full query_opts on search_text; msgpack point/id batch shapes matching the SDK wire types

## 2. Testing
- [ ] 2.1 C smoke test (open → create → upsert_batch → search → scroll → close) under ASan/Valgrind — zero leaks
- [ ] 2.2 16-thread TSan concurrency smoke on one vl_db
- [ ] 2.3 Header golden-file drift test green

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
