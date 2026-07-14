## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-002 §6, SPEC-003 WAL-043; DAG T2.5, T2.6, T2.9
- [x] 1.2 fs4 advisory locking: exclusive rw / shared ro, immediate Locked on conflict (STG-060)
- [x] 1.3 read_only open: ReadOnly on mutating calls, WalPending unless read_only_ignore_wal (STG-062)
- [x] 1.4 Damaged-tail tolerance: ro and rw open succeed when damage is beyond the committed TOC (STG-003)

## 2. Testing
- [x] 2.1 Second-open integration test: second writer gets Locked; ro also Locked while rw held (persistence::second_open_gets_locked)
- [x] 2.2 Damaged-tail fixture: both open modes succeed reading committed state (persistence::damaged_tail_opens_in_both_modes)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (CHANGELOG, README locking/read-only note)
- [x] 3.2 Write tests covering the new behavior (4 new persistence tests: lock, read-only, WalPending, damaged-tail)
- [x] 3.3 Run tests and confirm they pass (cargo test: all suites green; clippy clean; wasm32 builds)

<!-- mmap-as-primary-store and HNSW-graph persistence (original 1.2/1.3, tests 2.1/2.3)
     are tracked in phase2f_mmap-hnsw-persistence; ADR-0003 records why hnsw_rs blocks them. -->
