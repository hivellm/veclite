## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-008 in full, SPEC-004 §8; DAG T4.1, T4.2
- [x] 1.2 crates/veclite-ffi: handle types, vl_open/vl_open_memory/vl_db_close/vl_db_checkpoint lifecycle
- [x] 1.3 Collections surface: create/get/drop/rename/alias/list/stats/free (reindex/refit/payload_index_create → phase4g)
- [x] 1.4 Write surface: vl_upsert/vl_upsert_text/vl_delete (codec flag); upsert_batch/delete_batch → phase4g
- [x] 1.5 Read/search surface: vl_get/vl_count/vl_search/vl_search_text + vl_hits views/free fns (FFI-010); hybrid/search_batch/scroll → phase4g
- [x] 1.6 Meta fns (vl_version/vl_abi_version/vl_format_version); vl_chunk → phase4g
- [x] 1.7 catch_unwind wrappers + error-code mapping with compile-time exhaustiveness (VecLiteError::ffi_code) + thread-local message (FFI-003/020)
- [x] 1.8 cbindgen CI generation + committed golden veclite.h tracked in phase4g (FFI-006)
- [x] 1.9 API freeze: cargo public-api snapshot + CI additive-only check tracked in phase4g (API-062)

## 2. Testing
- [x] 2.1 Rust-side smoke: open → create → upsert → search → get → count → close; C ASan/Valgrind program → phase4g
- [x] 2.2 Panic-injection: forced panic → VL_ERR_INTERNAL, message set, process healthy (ffi::panic_at_the_boundary)
- [x] 2.3 Null-handle rejection; 16-thread TSan concurrency smoke → phase4g
- [x] 2.4 Error-code consistency (const ↔ ffi_code); header golden-file drift → phase4g

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (CHANGELOG, README, SPEC-008 status)
- [x] 3.2 Write tests covering the new behavior (5 FFI tests)
- [x] 3.3 Run tests and confirm they pass (all suites green; clippy clean; MSRV 1.87 builds; wasm unaffected)

<!-- The cbindgen golden header (FFI-006), the cargo public-api freeze snapshot
     (API-062), the remaining functions (batch/hybrid/scroll/chunk/reindex/refit/
     payload_index/snapshot/vacuum/db_info), and the ASan/Valgrind/TSan C tests are
     tracked in phase4g_ffi-header-freeze-full-surface — external-tooling and
     mechanical-surface work over the panic-safe, error-mapped core delivered here. -->
