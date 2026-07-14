## 1. Implementation
- [ ] 1.1 Context: re-read SPEC-006 FLT-020/030; review phase3a filter module + execute_query planner
- [ ] 1.2 Collection::create_payload_index(key, kind): backfill scan of existing payloads, build the index (was phase3a 1.5 late creation)
- [ ] 1.3 Journal PIDX_DECLARE (WAL op 8) on create_payload_index; wire its replay in apply_wal_entry; persist the declaration so the index rebuilds on open
- [ ] 1.4 HNSW over-fetch post-filter with adaptive growth for large/non-selective filtered queries; keep the exact pre-filter for selective candidate sets (was phase3a 1.6)

## 2. Testing
- [ ] 2.1 Late index creation: create_payload_index after upserts, then filtered search uses it; survives reopen
- [ ] 2.2 Over-fetch post-filter equals the scan baseline on a large corpus (FLT-031, identical results)

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
