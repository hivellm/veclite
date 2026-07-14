# Proposal: phase3e_filter-runtime-index-hnsw-prefilter

## Why

phase3a delivered the full payload-filter model, server-parity semantics, and
exact filtered execution (pre-filter over declared payload indexes; full scan
otherwise). Two items from SPEC-006 are intentionally split out because they are
optimizations/features on top of a correct core (results are already identical
with or without them — FLT-022/031):

1. Runtime index creation (FLT-020): indexes declared at collection creation
   work and rebuild on open, but a `create_payload_index` runtime call
   (journaled `PIDX_DECLARE`, WAL op 8, with a backfill scan of existing
   payloads) is not yet implemented.
2. HNSW over-fetch post-filter (FLT-030): filtered queries are currently exact
   brute-force over the index candidate set (selective filters) or the full live
   set (unindexed keys). This is correct but O(n) for large, non-selective
   filtered queries. The server's adaptive over-fetch strategy (HNSW search,
   then apply the filter, growing the fetch until `limit` results or candidates
   are exhausted) accelerates that case.

## What Changes

- `Collection::create_payload_index(key, kind)` — build the index by scanning
  existing payloads, journal `PIDX_DECLARE`, and persist the declaration so it
  rebuilds on open. Replay wires the WAL op (currently a no-op in
  `apply_wal_entry`).
- Filtered execution planner: when the candidate set is large (or absent) and
  the collection is big, use HNSW over-fetch + post-filter with adaptive growth
  instead of a full scan; keep the exact pre-filter for selective candidate
  sets. Results MUST stay identical to the scan baseline (FLT-031) —
  property-tested.

## Impact

- Affected specs: SPEC-006 FLT-020/030
- Affected code: crates/veclite/src/collection.rs (execute_query planner,
  create_payload_index), src/database.rs (PIDX_DECLARE replay), src/filter/index.rs
- Breaking change: NO (additive API; identical results)
- User benefit: add indexes without recreating a collection; fast filtered
  search on large collections with broad filters

## Note

Filtered `scroll` (FLT-032) is delivered alongside `scroll` itself in
phase3d_text-api-aliases-scroll, since scroll does not exist before then.
