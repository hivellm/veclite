## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-002 §6, SPEC-003 WAL-043; DAG T2.5, T2.6, T2.9
- [ ] 1.2 mmap read path over VECTORS segments with stride addressing; auto threshold 64 MiB (OpenOptions::mmap)
- [ ] 1.3 HNSW segment load; rebuild-from-vectors fallback emitting OpenOptions warning (STG-063)
- [ ] 1.4 fd-lock advisory locking: exclusive rw / shared ro, immediate Locked on conflict (STG-060)
- [ ] 1.5 read_only open: ReadOnly on mutating calls, WalPending unless read_only_ignore_wal (STG-062)
- [ ] 1.6 Damaged-tail tolerance: ro and rw open succeed when damage is beyond the committed TOC (STG-003)

## 2. Testing
- [ ] 2.1 Larger-than-RAM smoke: dataset 4x available RAM opens and serves searches via mmap
- [ ] 2.2 Two-process integration test: second writer gets Locked; ro + rw coexistence matrix
- [ ] 2.3 Corrupt-HNSW fixture: open rebuilds graph, warning fired, search results correct
- [ ] 2.4 Damaged-tail fixture: both open modes succeed reading committed state

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
