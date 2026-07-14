# Proposal: phase2b_wal-recovery-checkpoint

## Why
DAG T2.3 + T2.4: durability is VecLite's headline promise — kill -9 never corrupts the file. This cycle delivers the WAL sidecar, the three durability modes, recovery replay, and the checkpoint that moves WAL state into sealed segments (FR-07, FR-51, FR-52).

## What Changes
- WAL sidecar <db>.veclite-wal with 16-byte header (magic VLWL, uuid prefix guard) — WAL-001/002
- Entry format (seq, coll_id, op, crc32, MessagePack body) for the 8 op types — SPEC-003 §3
- Durability::Full/Normal/Off fsync policies — WAL-020/021
- Checkpoint: seal deltas → commit protocol → truncate WAL; triggers (size threshold, explicit, close) — WAL-030..032
- Recovery: replay in seq order, torn-tail discard, stale-WAL detection — WAL-040..043
- Close semantics: checkpoint-on-close, clean_close flag, idempotent close — WAL-050/051

## Impact
- Affected specs: SPEC-003 (all), SPEC-002 §5 integration
- Affected code: crates/veclite/src/storage/{wal,checkpoint,recovery}.rs
- Breaking change: NO
- User benefit: acked writes survive crashes; file integrity guaranteed in every durability mode
