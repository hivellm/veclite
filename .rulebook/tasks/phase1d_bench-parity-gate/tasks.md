## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-015 §4–5, docs/PRD.md NFR-01..04 and OQ-1; DAG T1.6, T1.7
- [ ] 1.2 Pin the standard benchmark corpus (dataset + queries, committed or fetched-by-hash)
- [ ] 1.3 Criterion benches: search p50, index build, batch insert (tests/bench/)
- [ ] 1.4 Parity harness: load corpus into VecLite + pinned Vectorizer server, compare top-10 (tests/compat/)
- [ ] 1.5 CI wiring: scaled-down smoke bench per PR (+-20% fence), full bench nightly
- [ ] 1.6 Document reference hardware profile; update PRD §12 marking OQ-1 resolved

## 2. Testing
- [ ] 2.1 Bench targets met on reference profile: p50 < 3 ms, build <= 2x server
- [ ] 2.2 Parity: top-10 overlap >= 0.99; defaults asserted identical (TST-030)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass (gate G1 evidence attached to the PR)
