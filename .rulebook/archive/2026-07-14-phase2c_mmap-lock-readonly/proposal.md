# Proposal: phase2c_mmap-lock-readonly

## Why
DAG T2.5 + T2.6 + T2.9: larger-than-RAM datasets need the mmap read path, single-writer semantics need the advisory file lock, and read-only open completes the OpenOptions contract (FR-03, FR-04, FR-53, FR-54).

## What Changes
- mmap read path (memmap2): stride-addressed vector access straight from VECTORS segments; auto-on for files > 64 MiB (STG-004)
- HNSW graph load from the HNSW segment; missing/corrupt graph → rebuild-from-vectors fallback with warning callback (STG-063)
- Advisory lock via fd-lock: exclusive on read-write, shared on read-only, Locked fail-fast (STG-060)
- read_only open: refuse writes with ReadOnly, WalPending guard unless read_only_ignore_wal, survive damaged uncommitted tail (STG-062, WAL-043)

## Impact
- Affected specs: SPEC-002 §6 (STG-060/062/003), SPEC-003 WAL-043
- Affected code: crates/veclite/src/storage/pager.rs (lock_file, exclusive/shared open), src/persist/mod.rs (read_only + WalPending), src/database.rs + collection.rs (ReadOnly on writes, crash hook), tests/persistence.rs
- Breaking change: NO
- User benefit: single-writer safety (concurrent-process misuse fails fast with Locked instead of corrupting), read-only serving, and open tolerating a damaged uncommitted tail

## Scope note (ADR-0003)
The mmap read path and HNSW-graph persistence (original "What Changes" bullets
1–2, STG-004/STG-063) are blocked by the pinned `hnsw_rs =0.3.4` (in-RAM
vector copy + no stable graph serialization) and were split into
`phase2f_mmap-hnsw-persistence`. This task delivers the locking, read-only, and
damaged-tail bullets, which have no such dependency.
