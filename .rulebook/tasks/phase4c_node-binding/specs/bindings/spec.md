# Bindings Specification

## ADDED Requirements

### Requirement: Non-Blocking Node Binding
The Node package SHALL execute every potentially long operation as a napi AsyncTask on the libuv threadpool so the event loop is never blocked, providing synchronous twins for every async method (SPEC-010 NODE-011).

#### Scenario: Event loop stays live during bulk index
Given a 10 second bulk indexing operation running via upsertBatch
When a 10 ms interval timer runs concurrently
Then timer jitter stays at or below 5 ms throughout

### Requirement: Float32Array Zero-Copy Interop
Vectors MUST cross the boundary as Float32Array without copying on search and batch upsert inputs, and hit vectors MUST return as Float32Array views released via external-buffer finalizers (SPEC-010 NODE-012).

#### Scenario: Search borrows the query buffer
Given a Float32Array query vector
When search is called
Then the native layer reads the buffer in place without allocating a copy
