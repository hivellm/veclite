"""Async facade for VecLite (SPEC-009 PY-031).

The same surface as the synchronous API, with ``async`` methods that run the
blocking core on the default asyncio thread-pool executor (via
:func:`asyncio.to_thread`). Because the Rust core releases the GIL around every
call (PY-030), these coroutines run truly concurrently. Synchronous
``veclite.Database`` remains the primary API; this module imports lazily.

    import asyncio
    import veclite

    async def main():
        db = veclite.aio.memory()
        docs = await db.create_collection("docs", dimension=3)
        await docs.upsert("a", [1.0, 0.0, 0.0])
        hits = await docs.search([1.0, 0.0, 0.0], limit=5)

    asyncio.run(main())
"""

import asyncio

from . import _veclite

__all__ = ["open", "memory", "AsyncDatabase", "AsyncCollection"]


async def open(path):
    """Open (or create) a durable single-file database off the event loop."""
    inner = await asyncio.to_thread(_veclite.Database.open, path)
    return AsyncDatabase(inner)


def memory():
    """Open an ephemeral in-memory database (construction never blocks)."""
    return AsyncDatabase(_veclite.Database.memory())


class AsyncCollection:
    """Async wrapper over :class:`veclite.Collection`; every data method runs on
    the thread pool so concurrent awaits overlap in the GIL-free core."""

    __slots__ = ("_inner",)

    def __init__(self, inner):
        self._inner = inner

    async def upsert(self, id, vector, payload=None, sparse=None):
        return await asyncio.to_thread(self._inner.upsert, id, vector, payload, sparse)

    async def upsert_batch(self, ids, vectors, payloads=None):
        return await asyncio.to_thread(self._inner.upsert_batch, ids, vectors, payloads)

    async def upsert_text(self, id, text, payload=None):
        return await asyncio.to_thread(self._inner.upsert_text, id, text, payload)

    async def get(self, id):
        return await asyncio.to_thread(self._inner.get, id)

    async def delete(self, id):
        return await asyncio.to_thread(self._inner.delete, id)

    async def search(
        self,
        vector,
        limit=10,
        ef_search=None,
        with_payload=True,
        with_vector=False,
        filter=None,
    ):
        return await asyncio.to_thread(
            self._inner.search,
            vector,
            limit,
            ef_search,
            with_payload,
            with_vector,
            filter,
        )

    async def search_text(self, query, limit=10):
        return await asyncio.to_thread(self._inner.search_text, query, limit)

    async def hybrid_search(self, vector, sparse, limit=10, alpha=0.5, rrf_k=60.0):
        return await asyncio.to_thread(
            self._inner.hybrid_search, vector, sparse, limit, alpha, rrf_k
        )

    async def scroll(self, limit=100, offset_id=None, filter=None):
        return await asyncio.to_thread(self._inner.scroll, limit, offset_id, filter)

    async def refit(self):
        return await asyncio.to_thread(self._inner.refit)

    async def stats(self):
        return await asyncio.to_thread(self._inner.stats)

    def __len__(self):
        return len(self._inner)


class AsyncDatabase:
    """Async wrapper over :class:`veclite.Database`. Collection-returning methods
    hand back :class:`AsyncCollection` so the whole chain stays awaitable."""

    __slots__ = ("_inner",)

    def __init__(self, inner):
        self._inner = inner

    async def create_collection(
        self,
        name,
        dimension,
        metric="cosine",
        quantization_bits=None,
        embedding_provider=None,
    ):
        inner = await asyncio.to_thread(
            self._inner.create_collection,
            name,
            dimension,
            metric,
            quantization_bits,
            embedding_provider,
        )
        return AsyncCollection(inner)

    def collection(self, name):
        # Handle lookup is a cheap in-memory map read; no thread hop needed.
        return AsyncCollection(self._inner.collection(name))

    async def delete_collection(self, name):
        return await asyncio.to_thread(self._inner.delete_collection, name)

    async def list_collections(self):
        return await asyncio.to_thread(self._inner.list_collections)

    async def create_alias(self, alias, target):
        return await asyncio.to_thread(self._inner.create_alias, alias, target)

    async def delete_alias(self, alias):
        return await asyncio.to_thread(self._inner.delete_alias, alias)

    async def aliases(self):
        return await asyncio.to_thread(self._inner.aliases)

    async def checkpoint(self):
        return await asyncio.to_thread(self._inner.checkpoint)

    def register_embedder(self, name, obj):
        # Registration stores the object; the embedding calls themselves are what
        # run under the released GIL later. Cheap and synchronous.
        return self._inner.register_embedder(name, obj)
