## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-010 in full, SPEC-004 for the mirrored surface; DAG T4.4
- [ ] 1.2 crates/veclite-node skeleton: napi-rs, platform-package layout, version fns (NODE-001/003)
- [ ] 1.3 Database/Collection classes: open/memory/createCollection options objects (NODE-010)
- [ ] 1.4 Async AsyncTask wrappers for all ops + *Sync twins (NODE-011)
- [ ] 1.5 Float32Array zero-copy in (search/upsert/upsertBatch flat) and out (hit vectors with finalizers) (NODE-012)
- [ ] 1.6 Handle lifecycle: close() promise, pending-op Closed rejection, emitWarning on leaked handles (NODE-013)
- [ ] 1.7 VecLiteError with code strings; message parity with Rust (NODE-020)
- [ ] 1.8 .d.ts generation; quickstart compiles under tsc --strict (NODE-003)

## 2. Testing
- [ ] 2.1 Event-loop-liveness: 10 s bulk index keeps a timer ticking at <= 5 ms jitter
- [ ] 2.2 Zero-copy proof for Float32Array paths (allocation tracking)
- [ ] 2.3 Error-code mapping tests; async and sync twins behave identically
- [ ] 2.4 Quickstart e2e on Node 18/20/22 and Bun from built prebuilds

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
