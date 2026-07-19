# Reverse: Vectorizer → VecLite

Take a slice of a Vectorizer server offline — a dev fixture, an edge
distribution, a per-tenant extract. The `veclite import` command reads the
server's storage directly and writes a single `.veclite` file.

## The flow

```bash
# Import selected collections from a server data set into a new .veclite file.
veclite import ./data/vectorizer.vecdb --collections docs,notes --out app.veclite
```

The source can be a `.vecdb` archive, a legacy `*_vector_store.bin` file, or a
directory containing either — the importer detects the layout automatically
(SPEC-013 IOP-020). Drop `--collections` to import everything; add `--force` to
overwrite an existing output.

## The degradation matrix — warnings, never silent

Server-only aspects that VecLite (a single-process embedded engine) does not
model are dropped **with an explicit warning on stderr**, never silently and
never fatally — with one exception, encryption, which refuses the import:

| Server aspect | Import behavior |
|---|---|
| Owner / tenant metadata | dropped, warning |
| Sharded collections | merged into one, warning |
| Graph edges / relationships | dropped, warning (VecLite has no graph model) |
| Text-normalization policy | dropped, warning (stored payloads kept verbatim) |
| HNSW level seed | dropped, warning |
| Server-only quantization (PQ, non-1/2/4/8-bit SQ) | imported without quantization — vectors stay exact f32, warning |
| **Encrypted payloads** | **refused with a clear error** — VecLite cannot decrypt |

### Server-only embedding providers become BYO-vector

A collection whose embedding provider this VecLite build cannot run (a server
neural provider such as candle or OpenAI) imports as a **bring-your-own-vector**
collection: the vectors and payloads are intact and searchable, text
re-embedding is disabled, and the origin provider id is recorded in the
collection config (`origin_provider`) so a later [graduation](graduation.md)
restores it. Unsupported filter-model features in imported payload-index
declarations (geo, nested) are dropped with a warning; the payload data itself
is preserved verbatim (IOP-023).

## After import

The result is an ordinary `.veclite` file — inspect it, verify its integrity,
and use it exactly like any other database:

```bash
veclite inspect app.veclite --json     # confirm collections, counts, config
veclite verify  app.veclite            # full integrity pass (exit 1 on damage)
```

A second export → re-import cycle is **stable** (no drift), so a slice can move
back and forth between the server and the edge without accumulating error — this
is a tested property of the round-trip gate (SPEC-013 §4.2).
