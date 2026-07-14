# Proposal: phase2a_pager-segments

## Why
DAG T2.1 + T2.2 start the format-v1 storage layer: the 4 KiB header with root-pointer swap and the immutable segment codec are the foundation every other persistence feature (WAL, mmap, snapshot, vacuum) builds on (FR-50, FR-53, FR-55).

## What Changes
- Pager: 4 KiB header (magic VECL, format_version, min_reader_version, header crc, flags, toc pointer, file uuid) per SPEC-002 §2
- Atomic header rewrite discipline (single 4 KiB write + fsync) — STG-011
- Segment codec: all 9 segment types with 32-byte headers, per-segment crc32, LZ4/zstd bodies (VECTORS never compressed) — SPEC-002 §3
- TOC document (MessagePack, generation counter, live-segment lists) + deterministic replay order — SPEC-002 §4
- Commit protocol: segments → fsync → TOC → fsync → header swap → fsync (STG-050)

## Impact
- Affected specs: SPEC-002 §1–5 (OQ-5 already resolved: MessagePack everywhere)
- Affected code: crates/veclite/src/storage/{pager,segment,toc}.rs
- Breaking change: NO
- User benefit: crash-consistent single-file foundation; torn writes can never damage committed state
