## 1. Implementation
- [ ] 1.1 Database.register_embedder(name, obj): PyEmbedder shim calling a Python callable under the GIL, exception chaining (PY-013)
- [ ] 1.2 Collection.scroll(after, limit, filter) -> (points, next_cursor)
- [ ] 1.3 veclite.aio thread-pool facade, lazily imported (PY-031)
- [ ] 1.4 abi3 wheel CI matrix (FR-66 platforms) + maturin build job

## 2. Testing
- [ ] 2.1 Zero-copy proof: tracemalloc asserts no O(n*dim) allocation on the ndarray search/upsert_batch paths (PY-020)
- [ ] 2.2 GIL-release: 8 concurrent search threads exceed 4x single-thread throughput (PY-030)
- [ ] 2.3 Quickstart e2e in a fresh venv from a built wheel; aio smoke under asyncio

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
