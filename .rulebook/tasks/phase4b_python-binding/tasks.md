## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-009 in full, SPEC-004 for the mirrored surface; DAG T4.3
- [ ] 1.2 crates/veclite-py skeleton: PyO3 abi3, maturin project, __version__/format_version (PY-001/003)
- [ ] 1.3 Database/Collection classes: open/memory kwargs, create_collection options, context managers (PY-010/012)
- [ ] 1.4 CRUD + search + hybrid + scroll surface with dict payloads and filter dicts
- [ ] 1.5 NumPy zero-copy paths: borrow on search, (n, dim) float32 batch upsert; lists fallback (PY-020..022)
- [ ] 1.6 GIL release around core calls (PY-030)
- [ ] 1.7 Exception hierarchy mapping every VecLiteError variant (PY-040)
- [ ] 1.8 register_embedder for Python objects with exception chaining (PY-013)
- [ ] 1.9 veclite.aio facade, lazy import (PY-031)

## 2. Testing
- [ ] 2.1 Zero-copy proof: allocation tracking asserts no O(n*dim) copies on ndarray paths
- [ ] 2.2 GIL-release test: 8 concurrent search threads > 4x single-thread throughput
- [ ] 2.3 Exception mapping tests for every variant; message equality with Rust
- [ ] 2.4 Quickstart e2e in a fresh venv from a built wheel; aio smoke under asyncio

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
