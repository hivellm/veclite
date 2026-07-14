# Proposal: phase4g_ffi-header-freeze-full-surface

## Why

phase4a delivered the C ABI core: the veclite-ffi crate (cdylib + staticlib),
panic-safe entry points (catch_unwind → VL_ERR_INTERNAL), error codes 1:1 with
VecLiteError (exhaustive in the core crate), handles + vl_buf/vl_hit_view, the
codec flag, and a strong function subset (lifecycle, collections, aliases,
writes, get/search/search_text, results, meta) with Rust-side tests. What
remains needs external tooling or is mechanical surface:

1. cbindgen golden header (FFI-006): generate veclite.h in CI and commit it;
   a drift test fails the build when the header changes without a version bump.
2. cargo public-api freeze snapshot (API-062, the T4.1 Rust-API freeze event):
   commit the public API surface and fail CI on non-additive changes.
3. The remaining functions: vl_upsert_batch/vl_delete_batch (codec-decode
   points/ids), vl_hybrid_search, vl_search_batch (+ vl_hits_batch), vl_scroll
   (+ vl_page), vl_chunk, vl_collection_reindex/refit, vl_payload_index_create,
   vl_db_snapshot/vacuum/db_info; full query_opts on vl_search_text.
4. Sanitizer C smoke tests: a real C program (open → create → upsert_batch →
   search → scroll → close) under ASan/Valgrind (zero leaks) and 16-thread TSan.

## What Changes

- build.rs/CI cbindgen step + committed crates/veclite-ffi/include/veclite.h.
- A cargo public-api snapshot committed + a CI additive-only job.
- The remaining extern functions + their result types/free fns.
- A tests/c/ smoke program run under the sanitizers in CI.

## Impact

- Affected specs: SPEC-008 FFI-006/030, §2 (full surface), SPEC-004 §8 (freeze)
- Affected code: crates/veclite-ffi/*, CI jobs
- Breaking change: NO (additive; the freeze prevents future breaks)
- User benefit: a committed, drift-checked header any language binds; the Rust
  API locked additive-only for 1.x; leak/thread-safety proven
