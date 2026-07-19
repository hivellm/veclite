# Graduation: VecLite → Vectorizer

The "SQLite → Postgres" moment. When an app outgrows one process — it needs
remote access, multi-process writers, auth, replication, or horizontal scale —
graduate its data to the [Vectorizer](https://github.com/hivellm/vectorizer)
server. Same math, same defaults, same concepts; **scoring is preserved**.

## The flow

```bash
# 1. Export the whole database (or a subset) to the server's storage format.
veclite export app.veclite --format vecdb --out ./export/
#    writes ./export/vectorizer.vecdb + ./export/vectorizer.vecidx (Compact layout)

# 2. Import into a running Vectorizer server (its CLI reads the archive).
vectorizer import ./export/vectorizer.vecdb

# 3. Swap the SDK: veclite → the Vectorizer client SDK.
#    Concepts carry over — same collection names, configs, provider ids, filters.
```

Export a named subset instead of the whole database:

```bash
veclite export app.veclite --format vecdb --out ./export/ --collections docs,notes
```

## What survives, unchanged

Everything that defines a query's answer travels intact (SPEC-013 IOP-011):

- **Vectors**, including quantized representations — translated **losslessly**,
  byte-identical encodings, no de-quantize / re-quantize round trip (IOP-001).
- **Payloads**, verbatim.
- **Collection configs** — dimension, metric, HNSW params, quantization,
  compression.
- **Declared payload-index kinds** — the server rebuilds the indexes.
- **Aliases**.
- **Embedding provider id + vocabulary** — a BM25 collection keeps **identical
  scoring** server-side (within 1e-5).

Tombstoned (deleted) data is never exported. The HNSW graph is exported only when
the server negotiates graph import via `.vecidx` metadata; otherwise the server
rebuilds it — either way the acceptance gate (below) is the arbiter.

## The acceptance guarantee

The graduation round-trip is a **tested gate**, not a hope: the standard
benchmark corpus exported from VecLite and imported into the pinned server
produces the same top-10 results — **overlap ≥ 0.99**, BM25 text scores identical
within **1e-5** (SPEC-013 §4, automated by `cargo xtask graduation`). The shared
conformance corpus runs in both repositories' gates, so a divergence fails the
build in whichever side changed.

## SDK familiarity

Migrating call sites is mechanical — the SDKs mirror each other:

| Concept | Vectorizer client | VecLite |
|---|---|---|
| Connect | `Client("vectorizer://host:port")` | `open("app.veclite")` |
| Create collection | `client.create_collection(name, config)` | `db.create_collection(name, options)` — same option names |
| Upsert | `client.insert(coll, vectors)` | `coll.upsert / upsert_batch` |
| Search | `client.search(coll, vec, k)` | `coll.search(vec, k)` |
| Hybrid | `client.hybrid_search(...)` | `coll.hybrid_query()...` |
| Filters | Qdrant-style JSON | identical JSON shape (v1 subset) |

What you gain on the server: auth, replication, sharding, and the network. What
you leave behind: the single-file simplicity. When you need a slice back offline,
see the [reverse guide](reverse-migration.md).
