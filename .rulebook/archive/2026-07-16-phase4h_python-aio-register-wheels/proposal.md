# Proposal: phase4h_python-aio-register-wheels

## Why

phase4b delivered the PyO3 core binding (Database/Collection, CRUD, search,
hybrid, NumPy zero-copy, GIL release, per-variant exception hierarchy) built as
an abi3 wheel via maturin and covered by pytest. The remaining SPEC-009 items
need extra tooling or are additive surface on top of the tested core:

1. register_embedder for Python objects (PY-013): a Rust Embedder shim that
   calls back into a Python callable, with exception chaining, exposed as
   Database.register_embedder(name, obj).
2. veclite.aio thread-pool facade (PY-031), lazily imported.
3. Collection.scroll on the Python side (cursor pagination).
4. Zero-copy allocation-tracking proof (PY-020 acceptance 1) and the 8-thread
   GIL-release throughput test (PY-030 acceptance 2 — aggregate > 4x single).
5. abi3 wheel CI matrix (build wheels for the FR-66 platforms) + a fresh-venv
   wheel install e2e test.

## What Changes

- Database.register_embedder + a PyEmbedder shim (holds a Py<PyAny>, calls its
  embed/fit under the GIL, maps Python exceptions to VecLiteError).
- A veclite/aio.py facade running blocking calls on a thread pool.
- Collection.scroll(after=None, limit=..., filter=None) -> (points, next_cursor).
- tests/ for allocation tracking (tracemalloc) and threaded throughput.
- A maturin CI job producing abi3 wheels + a wheel-install smoke test.

## Impact

- Affected specs: SPEC-009 PY-013/020/030/031, acceptance 1/2/4
- Affected code: crates/veclite-py/*, CI
- Breaking change: NO (additive)
- User benefit: async API, custom Python embedders, and pip-installable wheels
  proven leak-free and concurrent
