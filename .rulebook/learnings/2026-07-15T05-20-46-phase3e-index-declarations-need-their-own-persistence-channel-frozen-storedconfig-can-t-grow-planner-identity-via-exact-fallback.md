# phase3e: index declarations need their own persistence channel (frozen StoredConfig can't grow); planner identity via exact fallback
**Source**: manual
**Date**: 2026-07-15
**Related Task**: phase3e_filter-runtime-index-hnsw-prefilter
**Tags**: phase3e, payload-index, planner, wal, pidx, veclite
phase3e (runtime payload indexes + FLT-030 planner) findings:

1. LATENT BUG the task surfaced: persist/config.rs from_stored always restored payload_indexes = [] — even creation-time declarations were silently dropped on reopen (results stayed correct per FLT-022 accelerator semantics, so no test caught it). Lesson: accelerator-only state needs explicit persistence tests that observe the accelerator itself (added CollectionStats.payload_indexes for observability).

2. StoredConfig is FROZEN (positional MessagePack) — adding a field breaks decoding of old files. Two channels solved persistence without touching it: (a) checkpoint: one PIDX segment per key (SPEC-002 §3.1 reserved type 5; the phase2b PayloadIndex codec already existed unused — check body.rs before writing a new codec); (b) WAL: PIDX_DECLARE (op 8) journaled per declaration — including one per CREATION-TIME declaration right after CREATE_COLL, since the CreateColl body carries StoredConfig and thus no index list.

3. PIDX bitmaps are written (spec-shaped: kind byte + key + sorted postings over the COMPACTED slot numbering, rebuilt at seal time — never remap old bitmaps) but readers only harvest (key, kind) and rebuild bitmaps from payloads (FLT-021 rebuild model, same as HNSW/vocab).

4. compact() reset payload_indexes from config.payload_indexes — runtime declarations would vanish on vacuum. Preserve via declared() snapshot before reset. Pattern: any 'rebuild from config' site is a trap once runtime-mutable state exists.

5. FLT-030 planner shape that keeps FLT-031 exact: pre-filter (exact scoring over candidate set) when set*4 <= live OR live < 512 OR no graph; else HNSW over-fetch with ×4 adaptive growth collecting the distance-ordered matching prefix; on under-return (growth exhausted below limit) fall back to the FULL exact scan — approximation can then never gate results, and the 2k-corpus identity property test passes deterministically first try.

6. Idempotency contract: same-kind redeclare = Ok (matches WAL replay idempotency WAL-042); different kind = InvalidArgument; validate → log → apply ordering keeps conflicting declarations out of the WAL.