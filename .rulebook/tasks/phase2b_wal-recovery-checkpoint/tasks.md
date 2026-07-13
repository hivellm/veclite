## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-003 in full, SPEC-002 §5; DAG T2.3, T2.4
- [ ] 1.2 WAL file header + uuid-prefix guard against stale sidecars (WAL-001/002)
- [ ] 1.3 Entry codec for the 8 ops; whole-entry atomicity; seq monotonicity (WAL-010..012)
- [ ] 1.4 Durability modes wiring into every mutating call (WAL-020/021)
- [ ] 1.5 Checkpoint: seal in-memory deltas into segments, run commit protocol, truncate WAL only after header-swap fsync (WAL-030..032)
- [ ] 1.6 Checkpoint triggers: 64 MiB default threshold, explicit checkpoint(), clean close
- [ ] 1.7 Recovery replay on open: seq order, torn-tail discard, idempotent CREATE_COLL handling (WAL-040..042)
- [ ] 1.8 Close semantics: checkpoint + clean_close flag + lock release; Drop swallows error but leaves recoverable state (WAL-050/051)

## 2. Testing
- [ ] 2.1 Replay property tests: arbitrary op interleavings, crash at every entry boundary, replayed state == model
- [ ] 2.2 Torn-tail fuzz: corrupt/truncate last entry at every byte offset — open succeeds, prior entries intact
- [ ] 2.3 Crash-during-checkpoint: recovers to pre- or post-checkpoint state, never between (WAL-032)
- [ ] 2.4 Stale-WAL test: foreign WAL next to a copied db is ignored with warning

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
