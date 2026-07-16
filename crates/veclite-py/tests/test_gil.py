"""GIL-release throughput proof (SPEC-009 PY-030 / acceptance 2).

The core releases the GIL around every call (`py.allow_threads`), so searches
from multiple Python threads run in parallel. On a multi-core box, eight threads
must aggregate to more than 4x the single-thread search throughput. Skipped on
machines with too few logical CPUs to demonstrate it.
"""

import os
import threading
import time

import numpy as np
import pytest

import veclite

THREADS = 8
SEARCHES_PER_THREAD = 400


def _build_collection():
    db = veclite.Database.memory()
    c = db.create_collection("v", dimension=128, metric="cosine", quantization_bits=0)
    rng = np.random.default_rng(7)
    for i in range(2000):
        c.upsert(f"k{i}", rng.random(128, dtype=np.float32))
    return db, c


def _run_searches(c, query, n):
    for _ in range(n):
        c.search(query, limit=10, ef_search=200)


@pytest.mark.skipif(
    (os.cpu_count() or 1) < THREADS,
    reason=f"needs >= {THREADS} logical CPUs to demonstrate GIL-free scaling",
)
def test_search_scales_across_threads():
    db, c = _build_collection()
    query = np.ascontiguousarray(np.random.default_rng(8).random(128, dtype=np.float32))

    # Single-thread baseline: THREADS * SEARCHES_PER_THREAD searches serially.
    total = THREADS * SEARCHES_PER_THREAD
    t0 = time.perf_counter()
    _run_searches(c, query, total)
    single = time.perf_counter() - t0
    single_ops = total / single

    # Same total work spread across THREADS threads.
    workers = [
        threading.Thread(target=_run_searches, args=(c, query, SEARCHES_PER_THREAD))
        for _ in range(THREADS)
    ]
    t0 = time.perf_counter()
    for w in workers:
        w.start()
    for w in workers:
        w.join()
    parallel = time.perf_counter() - t0
    parallel_ops = total / parallel

    speedup = parallel_ops / single_ops
    # SPEC-030 acceptance: > 4x. We assert 4x; on a 32-core box it is far higher.
    assert speedup > 4.0, (
        f"threaded speedup {speedup:.1f}x (single {single_ops:.0f} ops/s, "
        f"parallel {parallel_ops:.0f} ops/s) — GIL not released?"
    )
    # Keep the handle alive until the assertions run.
    assert len(c) == 2000
    del db
