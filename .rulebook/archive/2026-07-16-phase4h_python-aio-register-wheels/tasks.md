## 1. Implementation
- [x] 1.1 Database.register_embedder(name, obj): PyEmbedder shim calling a Python callable under the GIL, exception chaining (PY-013) — embed/embed_batch/fit/export_state/import_state used when present; dimension cached at registration; raised exceptions chained as VecLiteError.__cause__
- [x] 1.2 Collection.scroll(after, limit, filter) -> (points, next_cursor) — already present as scroll(limit, offset_id, filter) -> {points, next_cursor} per the SPEC-009 example; verified by tests
- [x] 1.3 veclite.aio thread-pool facade, lazily imported (PY-031) — mixed maturin layout (veclite._veclite + python/veclite/{__init__,aio}.py); AsyncDatabase/AsyncCollection over asyncio.to_thread; PEP 562 lazy import
- [x] 1.4 abi3 wheel CI matrix (FR-66 platforms) + maturin build job — veclite-packaging.yml already builds the full FR-66 matrix; added a pytest step on native wheels

## 2. Testing
- [x] 2.1 Zero-copy proof: tracemalloc asserts no O(n*dim) allocation on the ndarray search/upsert_batch paths (PY-020) — test_zero_copy.py
- [x] 2.2 GIL-release: 8 concurrent search threads exceed 4x single-thread throughput (PY-030) — test_gil.py (skips under 8 logical CPUs)
- [x] 2.3 Quickstart e2e in a fresh venv from a built wheel; aio smoke under asyncio — wheel_e2e.sh (verified end-to-end) + test_aio.py

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [x] 3.1 Update or create documentation covering the implementation — crates/veclite-py/README.md (PyPI page) + SPEC-009 status
- [x] 3.2 Write tests covering the new behavior — 11 new pytest tests (register_embedder, aio, zero-copy, GIL) + wheel_e2e.sh
- [x] 3.3 Run tests and confirm they pass — 23 pytest passed in a fresh venv against the built wheel; conformance 34 cases green; clippy -D warnings + fmt clean
