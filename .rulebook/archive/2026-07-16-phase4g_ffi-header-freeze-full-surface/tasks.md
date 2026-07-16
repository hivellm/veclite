## 1. Implementation
- [x] 1.1 cbindgen header generation + committed golden veclite.h; drift test (FFI-006) — cbindgen dev-dep regenerates the header in-process and diffs it against the committed crates/veclite-ffi/veclite.h (tests/header_drift.rs)
- [x] 1.2 cargo public-api snapshot committed + CI additive-only check (API-062, T4.1 freeze) — crates/veclite/public-api.txt + `cargo xtask api-freeze [--bless]` gate + veclite-api-freeze.yml
- [x] 1.3 Remaining functions: upsert_batch/delete_batch, hybrid_search, search_batch (+vl_hits_batch), scroll (+vl_page), chunk, reindex/refit, payload_index_create, snapshot/vacuum/db_info — all added with SPEC-008 §2 signatures
- [x] 1.4 Full query_opts on search_text; msgpack point/id batch shapes matching the SDK wire types — search_text honors with_payload/with_vector (filter/ef_search routed to hybrid); search_batch takes the flat float array + shared query_opts; batch decode shapes match the SDK Point/id wire types

## 2. Testing
- [x] 2.1 C smoke test (open → create → upsert_batch → search → scroll → close) under ASan/Valgrind — zero leaks — tests/c/full_smoke.c + sanitize.sh (allocation-symmetric; verified functionally on Windows via zig cc)
- [x] 2.2 16-thread TSan concurrency smoke on one vl_db — tests/c/concurrency.c (16 threads × 64 writes on one shared handle; verified functionally = 1024 live)
- [x] 2.3 Header golden-file drift test green — regenerated after the surface grew; test passes

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [x] 3.1 Update or create documentation covering the implementation — docs/c-abi.md: freeze + sanitizer guarantees + complete function list
- [x] 3.2 Write tests covering the new behavior — 9 FFI integration tests (ffi_full_surface.rs) + drift test + api-freeze gate + 2 sanitizer C programs
- [x] 3.3 Run tests and confirm they pass — full workspace suite green; clippy -D warnings clean; fmt clean; api-freeze PASS
