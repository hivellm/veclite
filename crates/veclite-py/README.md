# hivellm-veclite

Embedded, single-file, in-process vector database — Python binding (SPEC-009).

The distribution is `hivellm-veclite` (plain `veclite` on PyPI is an unrelated
project); the import name is just `veclite`.

`pip install hivellm-veclite` installs a prebuilt **abi3** wheel (CPython 3.9+);
no Rust toolchain is needed. NumPy is optional — plain Python sequences work
everywhere — but recommended, since `float32` arrays are borrowed zero-copy.

```bash
pip install hivellm-veclite            # core
pip install "hivellm-veclite[numpy]"   # + numpy extra
```

Prebuilt wheels cover Windows x86_64 and Linux x86_64/aarch64. Other platforms
fall back to the sdist, which compiles from source and needs a Rust toolchain.

## Quickstart

```python
import veclite

db = veclite.Database.memory()            # or Database.open("data.veclite")
docs = db.create_collection("docs", dimension=3, metric="euclidean")
docs.upsert("a", [1.0, 0.0, 0.0], {"lang": "en"})
docs.upsert("b", [0.0, 1.0, 0.0])

hits = docs.search([0.9, 0.1, 0.0], limit=5)   # -> [{id, score, payload, vector?}]
page = docs.scroll(limit=100)                  # -> {points, next_cursor}
```

NumPy `float32` arrays are borrowed without a Python-side copy on `search` and
`upsert_batch` (PY-020); the GIL is released around every core call so searches
from multiple threads run in parallel (PY-030).

## Custom embedders

Register any Python object with `embed(text)` and a `dimension` property; auto-
embed collections then route text through it. `embed_batch`, `fit`,
`export_state`, and `import_state` are used when present. Exceptions raised in the
callback surface as a `VecLiteError` with the original exception chained
(`__cause__`).

```python
import numpy as np

class MyEmbedder:
    @property
    def dimension(self): return 384
    def embed(self, text: str) -> np.ndarray:
        ...  # return a float32 vector of length `dimension`

db.register_embedder("mine", MyEmbedder())
col = db.create_collection("t", dimension=384, embedding_provider="mine")
col.upsert_text("d1", "hello world")
hits = col.search_text("hello", limit=5)
```

A Python embedder is not persisted — only its serialized state. After reopening a
database that uses one, re-register it under the same name before use.

## Async (`veclite.aio`)

The optional `veclite.aio` facade mirrors the sync surface with `async` methods
that run the blocking core on the asyncio thread pool. Because the core is
GIL-free, awaits overlap. It imports lazily, so synchronous use pays nothing.

```python
import asyncio, veclite

async def main():
    db = veclite.aio.memory()
    docs = await db.create_collection("docs", dimension=3)
    await docs.upsert("a", [1.0, 0.0, 0.0])
    hits = await docs.search([1.0, 0.0, 0.0], limit=5)

asyncio.run(main())
```

## Errors

Every `veclite.VecLiteError` subclass (`CollectionNotFound`, `DimensionMismatch`,
`Locked`, `InvalidArgument`, `IoError`, …) carries the identical message as the
Rust core (PY-040). Catch the base `veclite.VecLiteError` to handle any of them.

## License

Apache-2.0.
