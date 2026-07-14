# Proposal: phase2d_snapshot-vacuum

## Why
DAG T2.7 + T2.8: append-mostly storage accumulates dead space; users need snapshot() for consistent backups and vacuum() to reclaim space in place — including the Windows unmap-truncate-remap path flagged as a project risk (FR-05, FR-06).

## What Changes
- snapshot(path): checkpoint, then copy header + live segments + fresh TOC into a new compacted standalone file with a new file_uuid; writers unblocked beyond the TOC-swap window (STG-070)
- vacuum(): checkpoint → rewrite live data of over-threshold collections → new TOC → truncate tail; pager handles unmap→truncate→remap on Windows (STG-071)
- Auto-vacuum escalation when tombstones exceed 25 percent of slots, tunable via OpenOptions (STG-072)

## Impact
- Affected specs: SPEC-002 §7
- Affected code: crates/veclite/src/storage/{snapshot,vacuum}.rs, pager remap support
- Breaking change: NO
- User benefit: single-file backups by copy; file size shrinks after bulk deletes on every OS
