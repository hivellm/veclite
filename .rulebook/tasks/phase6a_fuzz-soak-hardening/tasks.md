## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-015 §6; DAG T6.1, T6.2
- [ ] 1.2 cargo-fuzz targets: header/TOC/segment parser, WAL replay, filter parser, options decoding (TST-050)
- [ ] 1.3 Fuzz corpus seeding + regression corpus committed
- [ ] 1.4 Soak harness: write/search/vacuum/snapshot loop with invariant checks + RSS tracking (TST-051)
- [ ] 1.5 Memory-pressure configuration: mmap dataset 4x RAM
- [ ] 1.6 ASan + TSan CI runs of the integration suite (TST-052)
- [ ] 1.7 loom models for checkpoint/reader TOC-swap interleavings (CORE-054)

## 2. Testing
- [ ] 2.1 72 h accumulated fuzz clean across all targets
- [ ] 2.2 24 h soak: zero errors, RSS plateau (no leaks)
- [ ] 2.3 Sanitizers clean; loom models pass

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass (hardening evidence archived for the release PR)
