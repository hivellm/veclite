## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-009 in full, SPEC-004 for the mirrored surface; DAG T4.3
- [x] 1.2 crates/veclite-py skeleton: PyO3 abi3, maturin project, __version__/format_version (PY-001/003)
- [x] 1.3 Database/Collection classes: open/memory, create_collection kwargs, aliases (PY-010/012)
- [x] 1.4 CRUD + search + hybrid surface with dict payloads and filter dicts; scroll → phase4h
- [x] 1.5 NumPy zero-copy: borrow on search, (n, dim) float32 batch upsert; lists fallback (PY-020..022)
- [x] 1.6 GIL release around core calls via py.allow_threads (PY-030)
- [x] 1.7 Exception hierarchy mapping every VecLiteError variant, Rust-identical messages (PY-040)
- [x] 1.8 register_embedder for Python objects with exception chaining → phase4h (PY-013)
- [x] 1.9 veclite.aio facade → phase4h (PY-031)

## 2. Testing
- [x] 2.1 Zero-copy: numpy (n,dim) batch + numpy query search covered; tracemalloc allocation proof → phase4h
- [x] 2.2 GIL-release 8-thread throughput test → phase4h (allow_threads wired on every core call)
- [x] 2.3 Exception mapping tests for every reachable variant; message equality with Rust (tests/test_veclite.py)
- [x] 2.4 Quickstart e2e via maturin develop + pytest (8 tests); fresh-venv wheel + aio → phase4h

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (CHANGELOG, README, SPEC-009 status)
- [x] 3.2 Write tests covering the new behavior (8 pytest tests)
- [x] 3.3 Run tests and confirm they pass (8 pytest pass via maturin develop; py-crate clippy clean; workspace CI unaffected)

<!-- register_embedder (PY-013), the veclite.aio facade (PY-031), Python-side scroll,
     the tracemalloc zero-copy proof + 8-thread GIL throughput test, and the abi3
     wheel CI matrix are tracked in phase4h_python-aio-register-wheels — additive
     surface and external-tooling proofs over the tested core delivered here. -->
