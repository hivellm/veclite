# Proposal: phase2c_mmap-lock-readonly

## Why
DAG T2.5 + T2.6 + T2.9: larger-than-RAM datasets need the mmap read path, single-writer semantics need the advisory file lock, and read-only open completes the OpenOptions contract (FR-03, FR-04, FR-53, FR-54).

## What Changes
- mmap read path (memmap2): stride-addressed vector access straight from VECTORS segments; auto-on for files > 64 MiB (STG-004)
- HNSW graph load from the HNSW segment; missing/corrupt graph → rebuild-from-vectors fallback with warning callback (STG-063)
- Advisory lock via fd-lock: exclusive on read-write, shared on read-only, Locked fail-fast (STG-060)
- read_only open: refuse writes with ReadOnly, WalPending guard unless read_only_ignore_wal, survive damaged uncommitted tail (STG-062, WAL-043)

## Impact
- Affected specs: SPEC-002 §6, SPEC-003 WAL-043
- Affected code: crates/veclite/src/storage/{mmap,lock}.rs, open path in database.rs
- Breaking change: NO
- User benefit: datasets bigger than RAM page in on demand; concurrent-process misuse fails fast instead of corrupting
