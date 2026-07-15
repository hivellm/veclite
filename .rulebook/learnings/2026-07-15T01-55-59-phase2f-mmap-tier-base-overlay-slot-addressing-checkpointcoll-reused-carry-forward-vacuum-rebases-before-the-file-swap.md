# phase2f mmap tier: base+overlay slot addressing, CheckpointColl::reused carry-forward, vacuum rebases before the file swap
**Source**: manual
**Date**: 2026-07-15
**Related Task**: phase2f_mmap-hnsw-persistence
**Tags**: phase2f, mmap, larger-than-ram, storage, adr-0004, veclite
phase2f delivered STG-004/063/064 per ADR-0004 (which SUPERSEDES ADR-0003 — read hnsw_rs source before trusting an ADR premise: hnsw_rs 0.3.4 DOES have dump/reload (hnswio) and data mmap (DataMap/set_mmap_threshold); the real blocker is that its on-disk form is a directory of its own files, incompatible with single-file identity).

Architecture that worked (small diff, no format change):
1. Base+overlay: CollectionData.base (Vec<VectorsRegion> mmap windows covering slots 0..base_count) + overlay Vec<f32> indexed at (slot - base_count)*dim. One accessor pair (vector_at with a per-query scratch Vec, copy_vector) replaced every direct data.vectors[slot*dim..] site; score_slot took a scratch param so both tiers run the SAME simd kernels → bit-identical scores.
2. Tier split by OpenOptions::memory_budget (default 4 GiB of mapped vector bytes): under → rebuild HNSW from the map in 8192-chunks on open; over → index=None and the EXISTING no-index brute-force path serves exact k-NN from the map (zero changes to execute_query).
3. VectorsRegion: CRC the segment body ONCE at construction, then store absolute map offsets — never re-hash per access (a per-access CRC would be quadratic death). Holds Arc<FileMap> so lifetime is safe without self-references.
4. Clean carry-forward: CollectionData.dirty (set in apply_upsert/tombstone/compact) + BaseTier.seg_refs (the TOC entry's SegRefs). Checkpoint with allow_reuse=true emits CheckpointColl{reused: Some(refs), segments: []} → pager writes only the new TOC. CRITICAL: snapshot and vacuum write FRESH files → allow_reuse=false always (offsets would dangle).
5. Vacuum + Windows: compact() rebases mapped collections to RAM and sets base=None BEFORE Pager::replace_with's close→rename→reopen — Windows cannot rename over a mapped file. delete_collection also drops the base so a stale user handle can't pin the map.
6. Eligibility: auto-embed collections never map (vectors re-derived from _text on open → the map saves nothing).
7. Gotchas: RwLockWriteGuard cannot split field borrows — take `let data = &mut *guard;` first (plain &mut splits fine). A `#[cfg]` block as trailing expression is a statement (E0308 expected ()) — use `return` inside + #[allow(clippy::needless_return)]. fs4's whole-file LockFileEx does NOT block a second same-process read handle's mmap views (locks gate ReadFile/WriteFile, not mappings).

Known bounds (documented, not stubs): checkpoint of a MUTATED over-budget collection and vacuum/snapshot of one materialize the live set transiently; the at-scale 4×RAM run is DAG T6.2's deliverable.