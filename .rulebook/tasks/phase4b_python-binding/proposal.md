# Proposal: phase4b_python-binding

## Why
DAG T4.3: Python is the highest-volume ecosystem for the target personas (RAG, agents, notebooks). The binding must hit the SQLite bar — pip install with no Rust toolchain — with NumPy zero-copy as the headline perf feature (FR-61).

## What Changes
- crates/veclite-py: PyO3 direct binding (not over the C ABI), abi3 wheels via maturin, Python >= 3.9 (PY-001)
- API surface mirroring SPEC-004 in snake_case with kwargs options (PY-010..013)
- NumPy zero-copy: search borrows C-contiguous float32 buffers; upsert_batch takes (n, dim) arrays without per-row copies; NumPy optional (PY-020..022)
- GIL released around every core call (PY-030); veclite.aio thread-pool facade (PY-031)
- Exception hierarchy per variant with Rust-identical messages (PY-040)
- Custom Python embedders via register_embedder with traceback chaining (PY-013)

## Impact
- Affected specs: SPEC-009 (all)
- Affected code: crates/veclite-py/ (new), maturin config
- Breaking change: NO
- User benefit: pip install veclite → 5-line quickstart on a clean machine
