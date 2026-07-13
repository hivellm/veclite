# 07 — Vectorizer Compatibility & Graduation Path

## Compatibility contract

VecLite and Vectorizer share **concepts and math**, not wire formats or files:

| Layer | Shared? | Mechanism |
|---|---|---|
| Distance metrics, HNSW params, quantization encodings, SIMD kernels, compression | ✅ identical | both depend on `vectorizer-core` (same crate version line) |
| Collection config semantics (dimension, metric, m/ef, quantization, embedding_provider) | ✅ identical | VecLite `CollectionOptions` maps 1:1 to server `CollectionConfig` |
| Embedding provider names + vocabulary state (`bm25`, `tfidf`, `fastembed:<model>`) | ✅ identical | same provider IDs; vocabulary state translates both ways |
| Filter model (must/should/must_not; match/range) | ✅ subset | VecLite implements the v1 subset of the server's Qdrant-style filters (no geo/nested yet) |
| On-disk file | ❌ different | `.veclite` (single file) vs `.vecdb` + `.vecidx` + `snapshots/`; logical import/export bridges them |
| API/wire protocol | ❌ n/a | VecLite has no wire; SDK *shapes* mirror the server SDKs for familiarity |

Rule for both repos: **any change to quantization encodings, distance math, or HNSW serialization goes through `vectorizer-core`** so parity can't silently drift.

## Graduation path (VecLite → Vectorizer server)

The "SQLite → Postgres" moment. When an app needs remote access, multi-process writers, auth, replication, or horizontal scale:

```bash
# 1. Export each collection (or the whole db) to the server's storage format
veclite export app.veclite --format vecdb --out ./export/

# 2. Import into a running Vectorizer (server CLI, reads the exported archive)
vectorizer import ./export/vectorizer.vecdb

# 3. Swap the SDK: veclite → vectorizer client SDK
#    Concepts carry over: same collection names/configs/provider IDs/filter model.
```

What survives the migration unchanged: vectors (including quantized reps), payloads, payload indexes (rebuilt server-side from declared kinds), collection configs, aliases, embedding provider + vocabulary (BM25 collections keep identical scoring), HNSW parameters (graph itself is rebuilt or imported depending on server version support).

Acceptance target (from [01](01-vision-and-scope.md)): top-10 result overlap ≥ 0.99 between VecLite pre-export and server post-import on the standard benchmark set.

## Reverse path (Vectorizer → VecLite)

Supported for the "take a slice offline" workflow (dev fixtures, edge distribution, per-tenant extracts):

```bash
veclite import ./data/vectorizer.vecdb --collections docs,notes --out app.veclite
```

- Reads both server storage layouts (`detect_format`: Compact `.vecdb` and Legacy `*_vector_store.bin`).
- Server-only aspects are dropped with warnings, never errors: owner/tenant metadata, encryption policies (refuse if payload-encrypted — cannot decrypt), sharded collections (merged into one), graph edges (until VecLite grows graph support).
- Collections using server-only embedding providers (candle `real-models`, OpenAI) import as **BYO-vector** collections: vectors and payloads intact, text re-embedding disabled, with a recorded `origin_provider` for later graduation.

## SDK familiarity mapping

| Concept | Vectorizer SDK (client) | VecLite SDK (embedded) |
|---|---|---|
| Connect | `Client("vectorizer://host:15503")` | `open("app.veclite")` |
| Create collection | `client.create_collection(name, config)` | `db.create_collection(name, options)` — same option names |
| Upsert | `client.insert(coll, vectors)` / batch | `coll.upsert / upsert_batch` |
| Search | `client.search(coll, vec, k)` | `coll.search(vec, k)` |
| Hybrid | `client.hybrid_search(...)` | `coll.hybrid_query()...` |
| Filters | Qdrant-style JSON | identical JSON shape (v1 subset) |
| Auth / API keys | yes | absent by design |
| Replication / cluster admin | yes | absent by design |

Naming intentionally matches so that migrating call sites is mechanical.

## Repo relationship & shared-crate policy

- New repo: `hivellm/veclite`. Independent release cadence, own CHANGELOG, own semver.
- Depends on `vectorizer-core` from crates.io (published by the Vectorizer repo). Version policy: VecLite pins a minor line (`vectorizer-core = "3.5"`); breaking changes in core require a coordinated major.
- **No git submodules, no path deps across repos.** If VecLite needs a change in `vectorizer-core`, the PR lands in the Vectorizer repo first, publishes, then VecLite bumps.
- Post-1.0 evaluation ([08-roadmap.md](08-roadmap.md)): extract a shared `vectorizer-engine` crate (collection/HNSW/filter logic) used by both server and VecLite, eliminating the extract-and-adapt fork. Deferred until VecLite's API stabilizes — premature unification would couple release trains during VecLite's fastest-changing phase.

## Divergence policy

When VecLite and the server disagree on behavior for the same input (same collection config, same data, same query), it's a **bug in one of them** — tracked in a shared conformance corpus (`tests/compat/`) run in both repos' CI against golden results. Exceptions must be documented in this file with rationale (currently: none).
