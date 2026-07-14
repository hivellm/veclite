# 3. mmap-as-primary-store and HNSW-graph persistence deferred while hnsw_rs is the index

**Status**: accepted
**Date**: 2026-07-14
**Related Tasks**: phase2c_mmap-lock-readonly, phase2f_mmap-hnsw-persistence

## Context

SPEC-002 §6 (STG-004, FR-53) calls for serving datasets larger than RAM
through memory-mapped fixed-stride VECTORS segments, and STG-063 calls for
loading the HNSW graph from its segment (rebuilding from vectors on a checksum
miss). Phase2c set out to deliver both, alongside single-writer locking and
read-only open.

Two upstream facts about the pinned index crate (`hnsw_rs =0.3.4`, CORE-030)
make both goals unbuildable as specified without a prior architectural change:

1. **hnsw_rs keeps its own full f32 copy of every vector inside the graph, in
   RAM.** The graph and its vectors are resident regardless of how VECTORS
   segments are stored. Memory-mapping the VECTORS segments cannot make a
   dataset larger than RAM openable, because the index itself does not fit —
   mmap would only save the flat-store copy, not the dominant graph+vectors
   copy. A true larger-than-RAM path requires an index that reads vectors from
   the mmap (custom HNSW, or a flat/IVF index that scans mapped pages).

2. **hnsw_rs 0.3.4 offers no stable graph serialization.** The exact-version
   pin CORE-030 exists precisely because its on-disk graph format is not
   guaranteed across releases. We cannot persist and reload the HNSW segment
   byte-stably; today the graph is always rebuilt from vectors on open.
   STG-063's "load the graph, rebuild on corruption" contract collapses to
   "always rebuild", so there is no graph segment to corrupt or checksum yet.

Locking (STG-060), read-only open with the WalPending guard (STG-062), and
damaged-tail tolerance (STG-003) have NO such dependency and were fully
delivered and tested in phase2c.

## Decision

Ship phase2c as **locking + read-only + damaged-tail only**. Defer
mmap-as-primary-store (task 1.2 / test 2.1) and HNSW-graph persistence with
rebuild-fallback (task 1.3 / test 2.3) to a dedicated follow-up task
(**phase2f_mmap-hnsw-persistence**), which is gated on first choosing the index
strategy: either replace hnsw_rs with an index that reads vectors from the mmap
and has a stable serialized graph, or vendor/fork an HNSW that does.

Until that index decision is made, mmap of VECTORS segments and HNSW-segment
persistence add cost without delivering the larger-than-RAM or fast-open
benefits they exist for. The graph continues to be rebuilt from vectors on
every open (correct, just not yet fast for huge datasets).

## Consequences

**Positive**: phase2c ships real, tested value (single-writer safety, read-only
serving, corruption tolerance) without blocking on an index rewrite. The
deferral is captured with a concrete unblock condition rather than an open TODO.

**Negative**: datasets larger than RAM cannot be opened yet, and open time is
O(rebuild) rather than O(mmap graph) for large collections — both acceptable
for the current target sizes and both tracked in phase2f.

**Risk**: the eventual index swap is a larger change than a drop-in; phase2f
must begin with its own ADR weighing custom-HNSW vs flat/IVF vs a maintained
HNSW crate with stable serialization. **STG-004/STG-063 remain unsatisfied
until phase2f lands and MUST NOT be marked complete before then.**

## Alternatives Considered

- **Build mmap of VECTORS segments now anyway** — rejected: with hnsw_rs
  holding vectors in RAM it neither enables larger-than-RAM nor reduces
  resident memory, so it is pure cost with no user-visible benefit.
- **Persist the hnsw_rs graph bytes as-is** — rejected: the format is not
  stable across versions (the reason for the exact pin), so a persisted graph
  could fail to load after any dependency bump, which is worse than a
  deterministic rebuild.
- **Replace hnsw_rs in phase2c to unblock both items** — rejected: an index
  swap is a cross-cutting change deserving its own ADR and task; folding it
  into phase2c would violate task-scope and sequential-editing discipline.
