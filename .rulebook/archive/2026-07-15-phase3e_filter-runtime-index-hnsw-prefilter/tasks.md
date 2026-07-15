## 1. Implementation
- [x] 1.1 Context: re-read SPEC-006 FLT-020/030; review phase3a filter module + execute_query planner. Found and fixed a latent gap: index declarations were never persisted (from_stored restored an empty list), so even creation-time indexes were silently dropped on reopen.
- [x] 1.2 Collection::create_payload_index(key, kind): validates the key (non-empty, no '_' prefix, no nested path), same-kind redeclare is an idempotent no-op, kind conflict is InvalidArgument; backfills by scanning live payloads under the write lock (roaring set-inserts make re-inserts harmless). compact() now preserves runtime declarations (was resetting to the creation-time set).
- [x] 1.3 PIDX_DECLARE (WAL op 8) journaled on create_payload_index AND per creation-time declaration after CREATE_COLL (StoredConfig is frozen and carries no index list); replay wired in apply_wal_entry (idempotent). Declarations sealed as one PIDX segment per key (SPEC-002 §3.1 body: kind byte + key + sorted postings over the compacted slot numbering, reusing the phase2b PayloadIndex codec); load/load_based harvest declarations and rebuild bitmaps from payloads (FLT-021 rebuild model). CollectionStats.payload_indexes exposes the declared set.
- [x] 1.4 FLT-030 planner (filtered_planner, native): exact pre-filter over the index candidate set when selective (set*4 <= live) or the collection is small (< 512 live) or has no graph; otherwise HNSW over-fetch post-filter with adaptive ×4 growth until `limit` matches or the graph is exhausted, falling back to the exact scan on under-return (FLT-031 stays honest). wasm32 keeps the pure scan.

## 2. Testing
- [x] 2.1 Late index creation: backfill + immediate filtered use; declaration survives checkpoint+reopen (PIDX segment) and crash+replay (PIDX_DECLARE), for both runtime and creation-time declarations; redeclare/conflict/reserved-key semantics.
- [x] 2.2 Planner identity (FLT-031): 2 000-point corpus, all three strategies (selective pre-filter ~1%, non-selective post-filter ~90%, unindexed post-filter ~30%) return exactly a hand-computed scan baseline; empty-match filter agrees on empty.

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [x] 3.1 Update or create documentation covering the implementation — SPEC-006 status updated (FLT-020/030/031 delivered); CHANGELOG + README; rustdoc on create_payload_index/planner.
- [x] 3.2 Write tests covering the new behavior — 4 integration tests (tests/payload_index_runtime.rs).
- [x] 3.3 Run tests and confirm they pass — workspace suite 251 tests green; clippy -D warnings clean; wasm32 build green.
