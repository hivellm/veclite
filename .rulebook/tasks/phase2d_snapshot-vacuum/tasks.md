## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-002 §7; DAG T2.7, T2.8
- [ ] 1.2 snapshot(path): checkpoint + compacted copy from immutable segments; new file_uuid (STG-070)
- [ ] 1.3 vacuum(): rewrite live segments dropping tombstoned slots, rewrite IDDIR, swap TOC, truncate tail (STG-071)
- [ ] 1.4 Windows pager path: unmap → truncate → remap without invalidating concurrent readers
- [ ] 1.5 Auto-vacuum threshold (default 0.25) wired into checkpoint escalation (STG-072)

## 2. Testing
- [ ] 2.1 Snapshot-under-write: writers continue during snapshot; snapshot opens standalone with consistent state
- [ ] 2.2 File-shrink assertions: delete 50 percent, vacuum, file size drops accordingly
- [ ] 2.3 Windows CI: vacuum with active mmap passes (risk-table item)
- [ ] 2.4 Auto-vacuum trigger test at the tombstone threshold

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
