## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-008 in full, SPEC-004 §8; DAG T4.1, T4.2
- [ ] 1.2 crates/veclite-ffi: handle types, vl_open/vl_open_memory/vl_db_close + lifecycle surface
- [ ] 1.3 Collections surface: create/get/drop/rename/alias/list/stats/reindex/refit/payload_index_create
- [ ] 1.4 Write surface: vl_upsert/vl_upsert_batch/vl_upsert_text/vl_delete/vl_delete_batch (codec flag)
- [ ] 1.5 Read/search surface: vl_get/vl_count/vl_search/vl_search_text/vl_hybrid_search/vl_search_batch/vl_scroll + result views/free fns (FFI-010)
- [ ] 1.6 vl_chunk utility + meta fns (vl_version/vl_abi_version/vl_format_version)
- [ ] 1.7 catch_unwind wrappers + error-code mapping with compile-time exhaustiveness + thread-local message (FFI-003/020)
- [ ] 1.8 cbindgen CI generation + committed golden veclite.h (FFI-006)
- [ ] 1.9 API freeze: cargo public-api snapshot + CI additive-only check (API-062)

## 2. Testing
- [ ] 2.1 C smoke test (open → create → upsert_batch → search → scroll → close) under ASan/Valgrind — zero leaks
- [ ] 2.2 Panic-injection: every entry point forced to panic → VL_ERR_INTERNAL, message set, process healthy
- [ ] 2.3 Concurrency smoke: 16 threads on one vl_db under TSan
- [ ] 2.4 Header golden-file drift test

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
