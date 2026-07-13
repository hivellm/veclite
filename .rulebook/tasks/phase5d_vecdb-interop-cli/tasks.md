## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-013 and SPEC-014 in full; DAG T5.5, T5.6 and gate G5
- [ ] 1.2 Export writer: Compact .vecdb + .vecidx accepted by the server StorageReader; scope options (IOP-010..013)
- [ ] 1.3 Import reader: detect_format for Compact + Legacy; --collections subsetting (IOP-020/021)
- [ ] 1.4 Degradation matrix: tenant/shard/graph warnings, encrypted refusal, BYO fallback with origin_provider (IOP-022/023)
- [ ] 1.5 crates/veclite-cli: inspect/export/import/vacuum/snapshot/verify; exit codes 0/1/2/3; --json where offered (CLI-001..003)
- [ ] 1.6 verify command: full-file integrity pass naming damaged segments (CLI table)
- [ ] 1.7 Graduation round-trip automation vs dockerized pinned server (TST-032)
- [ ] 1.8 Wire the shared conformance corpus into both repos' CI (IOP-002)

## 2. Testing
- [ ] 2.1 Round-trip: export → server import → top-10 overlap >= 0.99; bm25 scores within 1e-5
- [ ] 2.2 Reverse round-trip stable on a second cycle (no drift)
- [ ] 2.3 Legacy-layout fixture imports correctly
- [ ] 2.4 Degradation fixtures for every matrix row; verify detects every bit-flip fixture
- [ ] 2.5 CLI exit-code contract integration tests; --help snapshots

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass (gate G5 evidence attached)
