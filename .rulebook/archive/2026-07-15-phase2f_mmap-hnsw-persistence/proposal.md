# Proposal: phase2f_mmap-hnsw-persistence

## Why

Phase2c delivered single-writer locking, read-only open, and damaged-tail
tolerance, but deferred two SPEC-002 §6 requirements because the pinned index
crate blocks them (ADR-0003): mmap-as-primary-store for larger-than-RAM
datasets (STG-004, FR-53) and HNSW-graph persistence with rebuild fallback
(STG-063). `hnsw_rs =0.3.4` keeps a full f32 copy of every vector in RAM and
has no stable graph serialization, so mmap of VECTORS segments neither enables
larger-than-RAM nor reduces memory, and the graph cannot be persisted
byte-stably. This task carries the deferred work and its blocking index
decision so STG-004/STG-063 can finally be satisfied.

## What Changes

- **Index strategy ADR first**: decide between (a) a vendored/forked HNSW that
  reads vectors from the mmap and serializes a stable graph, (b) a flat/IVF
  index over mapped pages, or (c) a maintained HNSW crate with stable
  serialization. This ADR is a prerequisite for every item below.
- mmap read path over VECTORS segments with fixed-stride addressing; auto-on
  above the 64 MiB threshold (`OpenOptions::mmap`, STG-004).
- HNSW graph load from the HNSW segment, with a rebuild-from-vectors fallback
  emitting the `OpenOptions` warning callback on a missing/corrupt graph
  (STG-063).
- Larger-than-RAM smoke test (dataset several times available RAM opens and
  serves searches via mmap) and a corrupt-HNSW fixture (open rebuilds, warning
  fired, results correct).

## Impact

- Affected specs: SPEC-002 §6 (STG-004, STG-063), PRD FR-53
- Affected code: `crates/veclite/src/storage/` (new mmap column reader), the
  index module (strategy swap), the open path in `database.rs`/`persist`
- Breaking change: NO (opt-in `mmap`; graph persistence is transparent)
- User benefit: datasets bigger than RAM page in on demand; large collections
  open without a full graph rebuild
- Blocked on: ADR-0003 (accepted) records the deferral; this task's own index
  ADR unblocks implementation
