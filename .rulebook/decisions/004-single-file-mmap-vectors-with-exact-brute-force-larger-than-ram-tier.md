# 4. Single-file mmap of VECTORS with an exact brute-force larger-than-RAM tier; HNSW rebuilt from mmap on open

**Status**: accepted
**Date**: 2026-07-14
**Related Tasks**: phase2f_mmap-hnsw-persistence
**Supersedes**: ADR-0003 (its deferral is now discharged; two of its premises are corrected below)

## Context

SPEC-002 §6 asks for two things the pinned index (`hnsw_rs =0.3.4`, CORE-030)
does not give us for free:

- **STG-004 / FR-53** — serve datasets larger than RAM through memory-mapped
  fixed-stride VECTORS segments.
- **STG-063 / FR-54** — load the HNSW graph from its segment on open, rebuilding
  from vectors on a checksum miss.

ADR-0003 deferred both to this task, on the belief that `hnsw_rs` offered no
path at all. Reading the `hnsw_rs 0.3.4` source directly corrects two of its
premises:

1. **hnsw_rs *does* serialize the graph** — `hnswio::HnswIo::file_dump(dir,
   basename)` dumps graph + data, and `load_hnsw` reloads it. The concern is
   cross-*version* format stability, not the absence of serialization; the exact
   pin (CORE-030) plus a version tag + CRC + rebuild fallback would handle that.
2. **hnsw_rs *does* mmap the vector data** — `datamap::DataMap` /
   `ReloadOptions::set_mmap_threshold` keep the upper graph layers resident and
   mmap the rest, which is a genuine larger-than-RAM path.

So the capability exists. The blocker is **architectural, not capability**:
hnsw_rs's dump/reload/mmap operate on **its own directory of files** with its own
on-disk magic (`MAGICDATAP`). Its mmap cannot be pointed at a byte range *inside*
our single `.veclite` file. Reusing it would force either a companion sidecar
directory or materializing the whole dataset to temporary files on open — both
break the **single-file guarantee** that is the centre of VecLite's identity
("one file, no server"), and both couple our durable state to hnsw_rs's
version-unstable on-disk format (exactly ADR-0003 premise #2, which remains
valid).

The frozen format already reserves `SegmentType::Hnsw` (byte 7), and VECTORS
segments are already fixed-stride and mmap-ready (STG-004/STG-030) — so no byte
format change is needed for either the chosen path or a future revisit.

## Decision

**Single-file purity. mmap the VECTORS segment as the primary vector store and
serve the larger-than-RAM tier by exact SIMD brute force; keep in-RAM HNSW for
datasets under a memory budget. Do not persist the hnsw_rs graph.**

Concretely:

- **mmap primary store (STG-004).** Open memory-maps the VECTORS segments and
  addresses vectors by stride with no decode. Auto-on above the 64 MiB threshold
  (`OpenOptions::mmap`), overridable.
- **Two search tiers, selected by a memory budget:**
  - *Fits the budget:* rebuild the HNSW graph in RAM from the mmap'd VECTORS via
    `parallel_insert` (rayon) on open, then serve ANN search as today. Rebuild is
    fast and deterministic; this is the "fast open" STG-063 exists for, delivered
    by parallel rebuild rather than by loading a persisted graph.
  - *Exceeds the budget:* skip the HNSW build entirely and serve exact k-NN by
    SIMD brute-force scan over the mmap'd fixed-stride VECTORS (vendored distance
    kernels). Recall is exact — a correctness *upgrade* over ANN, at O(n) per
    query.
- **STG-063 reframed (spec amendment, this task).** v1 does **not** persist the
  hnsw_rs graph — its on-disk format is version-unstable (ADR-0003 premise #2
  stands). The HNSW segment stays *reserved* but unused for graph persistence in
  v1; open always rebuilds from (mmap'd) vectors. The `OpenOptions` warning
  callback is retained in the API for a future persisted-graph path but is not
  fired in v1. This changes behaviour, not bytes — the freeze holds. SPEC-002
  STG-063 is amended in the same task (DAG §5 change control).

## Consequences

**Positive**: the single-file guarantee is preserved; larger-than-RAM works with
*exact* recall; no coupling to hnsw_rs's unstable on-disk format; reuses the
already-mmap-ready VECTORS layout and the vendored SIMD kernels; no byte-format
change, so the v1 freeze is untouched.

**Negative**: the larger-than-RAM tier is O(n·dim) per query (exact scan) rather
than sub-linear ANN — acceptable for the embedded target sizes, and exactness is
arguably a feature for that tier. Open is O(parallel rebuild) for the
fits-in-budget tier rather than O(load persisted graph); fast enough with rayon
for target sizes, and the warm-open budget (NFR-02, < 100 ms on the 1 M
reference file) is validated in this task's tests.

**Risk**: brute-force latency on very large datasets. Documented; the escape
hatch is a future custom mmap-native ANN index (see Alternative 3), which the
reserved HNSW segment and the frozen VECTORS layout already leave room for
without a format break.

## Alternatives Considered

- **hnsw_rs native dump/reload + DataMap mmap** — rejected: satisfies STG-004 and
  STG-063 literally (ANN even above RAM), but requires hnsw_rs's directory-of-
  files layout — a sidecar or temp-file materialization on open — which breaks
  the single-file identity and couples durable state to a version-unstable
  format.
- **Custom / forked mmap-native HNSW** reading vectors straight from our mmap'd
  VECTORS and serializing a stable graph into the HNSW segment — rejected *now*
  as disproportionate: it replaces the indexing core (high risk) for a benefit
  (sub-linear ANN above RAM) that the embedded target sizes do not yet require.
  Left as the post-1.0 escape hatch; the reserved HNSW segment keeps the door
  open without a byte-format change.
- **Persist the hnsw_rs graph bytes as-is into the HNSW segment** — rejected:
  the format is not stable across versions (the reason for the exact pin), so a
  persisted graph could fail to load after any dependency bump — worse than a
  deterministic rebuild (unchanged from ADR-0003).
