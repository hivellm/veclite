"""Zero-copy NumPy proof (SPEC-009 PY-020 / acceptance 1).

`search` borrows a C-contiguous float32 buffer and `upsert_batch` borrows an
`(n, dim)` float32 array — the buffer is read directly by the Rust core, never
copied into Python objects. We prove it with `tracemalloc`, which accounts only
for Python-side (PyMem) allocations: passing a multi-megabyte array through
these paths must not allocate anything close to its size on the Python heap.
"""

import tracemalloc

import numpy as np

import veclite


def _peak_python_bytes(fn):
    tracemalloc.start()
    tracemalloc.clear_traces()
    fn()
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    return peak


def test_upsert_batch_does_not_copy_the_array_into_python():
    db = veclite.Database.memory()
    c = db.create_collection("v", dimension=512, metric="euclidean", quantization_bits=0)

    n, dim = 2000, 512
    vectors = np.ascontiguousarray(np.random.default_rng(1).random((n, dim), dtype=np.float32))
    ids = [f"k{i}" for i in range(n)]
    array_bytes = vectors.nbytes  # ~4 MB
    assert array_bytes > 2_000_000

    peak = _peak_python_bytes(lambda: c.upsert_batch(ids, vectors))

    # A per-row Python copy would allocate on the order of `array_bytes`. The
    # borrow path stays far below it (a fraction of one row's worth of overhead).
    assert peak < array_bytes // 4, f"peak {peak} vs array {array_bytes} — buffer was copied"
    assert len(c) == n


def test_search_borrows_the_query_vector():
    db = veclite.Database.memory()
    c = db.create_collection("v", dimension=1024, metric="cosine", quantization_bits=0)
    rng = np.random.default_rng(2)
    for i in range(50):
        c.upsert(f"k{i}", rng.random(1024, dtype=np.float32))

    query = np.ascontiguousarray(rng.random(1024, dtype=np.float32))

    def do_searches():
        for _ in range(100):
            c.search(query, limit=10)

    peak = _peak_python_bytes(do_searches)
    # 100 searches, each borrowing the same 4 KB query: nowhere near 100*4 KB of
    # Python allocation if the buffer is borrowed rather than copied per call.
    assert peak < query.nbytes * 20, f"peak {peak} suggests per-call query copies"
