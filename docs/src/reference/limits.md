# Sizing & limits

VecLite is a single-process, single-file engine. The limits below are the hard
bounds the engine enforces; the sizing guidance is practical advice for staying
comfortably inside them.

## Hard limits

| Limit | Value | Enforced by |
|---|---|---|
| Vector dimension | `1..=65536` | `create_collection` rejects out-of-range |
| Collection name | ≤ 255 bytes | `create_collection` |
| Point id | ≤ 512 bytes | `upsert` |
| Payload | ≤ 16 MiB per point | `upsert` |
| File header | 4 KiB fixed | format v1 |
| Vectors per collection | `u64` slot space | practical limit is RAM / disk |

Vectors, ids, and payloads that violate a limit fail the call with a typed
`InvalidArgument` (or `DimensionMismatch`) error — never a panic, never a silent
truncation.

## Memory & the mmap tier

By default a collection's vectors live in RAM and searches run over the HNSW
graph. Under `OpenOptions::memory_budget` (default **4 GiB**), a collection
larger than the budget is served from a read-only memory map of the same file:

- **Within budget** — the HNSW graph is rebuilt from the map on open; search is
  the usual approximate-nearest-neighbour graph walk.
- **Past budget** — searches run as exact SIMD scans over the mapped vectors, so
  a dataset **larger than RAM** opens and serves with exact recall (at scan
  cost, linear in the collection size).

Set the budget to match the machine:

```rust
use veclite::{OpenOptions, VecLite};
let db = VecLite::open_with(
    "big.veclite",
    OpenOptions::new().memory_budget(8 * 1024 * 1024 * 1024), // 8 GiB
)?;
```

See [ADR-0004](../../../.rulebook/decisions/004-single-file-mmap-vectors-with-exact-brute-force-larger-than-ram-tier.md)
for the design.

## Sizing rules of thumb

- **Vector storage**: roughly `vectors × dimension × 4 bytes` for f32, or
  `× 1 byte` per dimension under the default SQ-8 quantization (the on-disk /
  interop encoding). A 1 M × 512 collection is ~2 GiB f32, ~0.5 GiB SQ-8.
- **HNSW graph**: proportional to `vectors × m` (default `m = 16`); budget a few
  hundred bytes per vector on top of the vector storage.
- **Payloads**: stored verbatim; size them like any JSON document store.
- **Deletes** tombstone; run [`vacuum`](../guides/cli.md) (or let auto-vacuum
  trigger past the tombstone threshold) to reclaim the space.

## WASM sizing

In the browser the whole database lives in the wasm linear memory (and,
optionally, an OPFS-persisted `.veclite` image). Keep client-side collections to
what fits a tab's memory budget — tens to low-hundreds of thousands of vectors
at modest dimension is comfortable; larger corpora belong on the server (see the
[graduation guide](../guides/graduation.md)). SPEC-012 has the detailed WASM
sizing guidance.
