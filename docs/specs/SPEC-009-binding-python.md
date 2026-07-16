# SPEC-009 — Python Binding (`veclite` on PyPI)

| | |
|---|---|
| **Status** | Implemented (phase4b + phase4h): the `veclite-py` PyO3 crate (abi3 wheel via maturin, mixed layout `veclite._veclite`), Database/Collection surface, NumPy zero-copy search + batch upsert, GIL release around every core call, a per-variant exception hierarchy with Rust-identical messages, `register_embedder` for custom Python embedders with chained tracebacks (PY-013), the lazily-imported `veclite.aio` async facade (PY-031), and `scroll`. Proven by the tracemalloc zero-copy test (PY-020), the 8-thread GIL-release throughput test (PY-030), and a fresh-venv wheel e2e; the abi3 wheel CI matrix (FR-66) runs the conformance corpus + pytest on every native build. |
| **Phase / tasks** | Phase 4 · T4.3 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-61, FR-65, FR-66 |
| **Planning source** | [06-sdk-bindings.md §Python](../vectorizer-lite/06-sdk-bindings.md) |

Requirement IDs `PY-xxx`. Binds the Rust crate directly via **PyO3** (not the C ABI). Behavior MUST match SPEC-004 semantics exactly; the conformance corpus (SPEC-015 §3) is the arbiter.

## 1. Packaging

- **PY-001** Package `veclite`, Python ≥ 3.9, **abi3** wheels built with maturin: manylinux + musllinux + macOS + Windows × x86_64/arm64 (FR-66). `pip install veclite` MUST never compile Rust.
- **PY-002** ONNX extra: `pip install veclite[onnx]` pulls the separate `veclite-onnx` wheel (EMB-040). Base wheel target ≤ 15 MB compressed.
- **PY-003** Version string == core crate version (lockstep, NFR-12); `veclite.__version__`, `veclite.format_version()`.

## 2. API surface

```python
import veclite

db = veclite.open("app.veclite", read_only=False, durability="normal",
                  mmap=None, background_checkpoint=False)     # kwargs mirror OpenOptions
db = veclite.memory()

docs = db.create_collection("docs", dimension=384, metric="cosine",
                            hnsw={"m": 16, "ef_construction": 200, "ef_search": 100},
                            quantization={"scalar": {"bits": 8}},
                            payload_indexes=[("lang", "keyword")])
notes = db.create_collection("notes", auto_embed="bm25", dimension=512)

docs.upsert("id-1", vector, payload={"lang": "en"})
docs.upsert_batch(points)                  # list[tuple[id, vector, payload?]] | numpy (n, dim) float32
notes.upsert_text("id-9", "text…", {"src": "readme"})

hits = docs.search(vector, limit=10,
                   filter={"must": [{"key": "lang", "match": "en"}]},
                   ef_search=200, with_payload=True, with_vector=False)
hits = notes.search_text("query", limit=5)
hits = docs.hybrid_search(dense=vec, sparse=(indices, values), alpha=0.5, limit=10)
page = docs.scroll(limit=100, offset_id="id-500")

db.snapshot("backup.veclite"); db.vacuum(); db.checkpoint()
db.close()
```

- **PY-010** Naming: snake_case mirroring SPEC-004 method-for-method. Enum-like options accept lowercase strings (`"cosine"`, `"euclidean"`, `"dotproduct"`; `"keyword"|"integer"|"float"`; `"full"|"normal"|"off"`).
- **PY-011** `Hit` is a small class with `id: str`, `score: float`, `payload: dict | None`, `vector: numpy.ndarray | None`; iterable/indexable results object.
- **PY-012** Context managers: `with veclite.open(...) as db:` and per-collection none (collections are lightweight views). `db.close()` idempotent; operations after close raise `veclite.Closed`.
- **PY-013** Custom embedders: `db.register_embedder("name", obj)` where `obj` implements `embed(str) -> list[float] | np.ndarray`, `dimension` property, optional `export_state/import_state` — wrapped into the Rust trait; exceptions inside the Python embedder surface as `VecLiteError` with the original traceback chained.

## 3. NumPy zero-copy (the headline perf feature)

- **PY-020** `search(np.ndarray)` borrows the buffer (C-contiguous float32; other dtypes/layouts converted with a documented copy). `upsert_batch` accepts an `(n, dim) float32` array without per-row copies.
- **PY-021** `with_vector=True` results expose vectors as NumPy arrays backed by the hit buffer (copy-on-write safety: freed when the results object drops — the array holds a reference).
- **PY-022** NumPy is an **optional** dependency: everything works with lists; NumPy paths activate when the argument is an ndarray.

## 4. Threading & async

- **PY-030** The GIL is released around every core call (`py.allow_threads`) — searches from multiple Python threads scale.
- **PY-031** Optional `veclite.aio` facade: same surface with `async` methods executing on a thread pool. Sync is the primary API; `aio` ships in the same wheel, imports lazily.

## 5. Errors

- **PY-040** Exception hierarchy: `veclite.VecLiteError(Exception)` base; subclasses per variant (`CollectionNotFound`, `DimensionMismatch`, `Locked`, `Corrupt`, `UnsupportedFormatVersion`, `UnsupportedProvider`, `ReadOnly`, `WalPending`, `Closed`, `InvalidArgument`); `OSError` chained for `Io`. Messages identical to the Rust display strings.

## 6. Acceptance criteria

1. Conformance corpus green (SPEC-015 §3) on the full wheel matrix.
2. Clean-machine test: fresh venv, `pip install veclite`, run the README quickstart — no Rust toolchain present (gate G4).
3. Zero-copy proof: benchmark asserting no O(n·dim) copy on ndarray `upsert_batch`/`search` (allocation tracking).
4. GIL-release test: concurrent searches from 8 threads achieve > 4× single-thread throughput on the reference profile.
5. `veclite.aio` smoke: quickstart under asyncio.
