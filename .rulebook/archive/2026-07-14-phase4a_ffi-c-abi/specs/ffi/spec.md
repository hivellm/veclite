# FFI Specification

## ADDED Requirements

### Requirement: Panic-Safe C ABI
Every FFI entry point SHALL be wrapped in catch_unwind mapping panics to VL_ERR_INTERNAL with the thread-local error message set, and error codes MUST map 1:1 to VecLiteError variants and never be renumbered within a major version (SPEC-008 FFI-003, §3).

#### Scenario: Internal panic contained
Given an FFI function whose internals are forced to panic in a test build
When the function is called from C
Then it returns VL_ERR_INTERNAL, vl_last_error_message returns a diagnostic string, and the process continues normally

### Requirement: Frozen Additive-Only Surface
The generated veclite.h MUST match the committed golden header in CI, and from this task onward the public Rust API and the C ABI SHALL evolve additive-only within the major version, enforced by a cargo public-api snapshot check (SPEC-008 FFI-006/007, SPEC-004 API-061/062).

#### Scenario: Non-additive change blocked
Given the committed API snapshot and golden header
When a PR removes or re-types an existing public symbol
Then CI fails the API-compatibility job
