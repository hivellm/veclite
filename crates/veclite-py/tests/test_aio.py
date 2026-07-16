"""Async facade (SPEC-009 PY-031 / acceptance 5): the quickstart under asyncio,
plus concurrent awaits overlapping in the GIL-free core."""

import asyncio

import numpy as np

import veclite


def test_aio_quickstart():
    async def main():
        db = veclite.aio.memory()
        docs = await db.create_collection("docs", dimension=3, metric="euclidean")
        await docs.upsert("a", [1.0, 0.0, 0.0], {"lang": "en"})
        await docs.upsert("b", [0.0, 1.0, 0.0])
        assert len(docs) == 2

        hits = await docs.search([0.9, 0.1, 0.0], limit=1)
        assert hits[0]["id"] == "a"
        assert hits[0]["payload"] == {"lang": "en"}

        got = await docs.get("a")
        assert got["vector"] == [1.0, 0.0, 0.0]
        assert await docs.delete("a") is True

        page = await docs.scroll(limit=10)
        assert {p["id"] for p in page["points"]} == {"b"}

    asyncio.run(main())


def test_aio_is_lazily_imported():
    # Accessing veclite.aio triggers the PEP 562 lazy import; it is the same
    # module object on repeat access.
    m1 = veclite.aio
    m2 = veclite.aio
    assert m1 is m2
    assert hasattr(m1, "AsyncDatabase")


def test_aio_concurrent_searches():
    async def main():
        db = veclite.aio.memory()
        c = await db.create_collection("v", dimension=8, metric="cosine")
        rng = np.random.default_rng(0)
        for i in range(200):
            await c.upsert(f"k{i}", rng.random(8, dtype=np.float32))

        q = rng.random(8, dtype=np.float32)
        # Fire many searches concurrently; gather awaits them on the thread pool.
        results = await asyncio.gather(*[c.search(q, limit=5) for _ in range(32)])
        assert len(results) == 32
        assert all(len(r) == 5 for r in results)

    asyncio.run(main())
