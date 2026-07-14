## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-002 §7; DAG T2.7, T2.8
- [x] 1.2 snapshot(path): checkpoint + compacted copy from live state; new file_uuid (STG-070)
- [x] 1.3 vacuum(): compact live in memory dropping tombstoned slots, rewrite to a fresh compacted generation, shrink the file (STG-071)
- [x] 1.4 Windows-safe file swap: crash-safe close→rename→reopen preserving file_uuid, readers served from memory so none are invalidated (STG-071)
- [x] 1.5 Auto-vacuum threshold (default 0.25) wired into checkpoint escalation (STG-072)

## 2. Testing
- [x] 2.1 Snapshot-under-write: writers continue during snapshot; snapshot opens standalone with consistent state (persistence::snapshot_is_standalone_and_consistent_under_writes)
- [x] 2.2 File-shrink: delete 50 percent, vacuum, file size drops; pager stays live after the swap (persistence::vacuum_shrinks_file_and_pager_survives)
- [x] 2.3 Windows CI: the vacuum shrink test exercises close→rename→reopen on the 3-OS matrix including Windows (persistence::vacuum_shrinks_file_and_pager_survives)
- [x] 2.4 Auto-vacuum trigger at the tombstone threshold (persistence::auto_vacuum_escalates_at_threshold)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (CHANGELOG, README, SPEC-002 §7 v1 note)
- [x] 3.2 Write tests covering the new behavior (snapshot, vacuum shrink, auto-vacuum)
- [x] 3.3 Run tests and confirm they pass (cargo test: all suites green; clippy clean; wasm32 builds)

<!-- The in-place append-then-truncate vacuum with an active mmap (unmap->truncate->remap,
     STG-071) and its active-mmap Windows vacuum test are tracked in
     phase2f_mmap-hnsw-persistence, which introduces the mmap read path (ADR-0003). -->
