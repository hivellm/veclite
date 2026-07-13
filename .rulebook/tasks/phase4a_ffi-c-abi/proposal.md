# Proposal: phase4a_ffi-c-abi

## Why
DAG T4.1 + T4.2: the C ABI is the contract for Go/C#/community bindings and its creation is the public Rust API freeze event — from here on the API is additive-only within 1.x (FR-60, API-060/061).

## What Changes
- veclite-ffi crate (cdylib + staticlib): full SPEC-008 §2 surface — handles, out-params, vl_buf/vl_hits, codec flag JSON/MessagePack (FFI-001..005)
- cbindgen header generation in CI + committed golden header (FFI-006)
- catch_unwind at every entry point; panic → VL_ERR_INTERNAL (FFI-003)
- Error-code table 1:1 with VecLiteError, compile-time exhaustiveness (SPEC-008 §3)
- Thread-local vl_last_error_message (FFI-020)
- API freeze: cargo public-api snapshot committed; CI fails non-additive changes (API-062, REL-032)

## Impact
- Affected specs: SPEC-008 (all), SPEC-004 §8 (freeze activated)
- Affected code: crates/veclite-ffi/ (new), CI header/api-snapshot jobs
- Breaking change: NO (it prevents future ones)
- User benefit: stable ABI any language can bind; Rust API contract locked for 1.x
