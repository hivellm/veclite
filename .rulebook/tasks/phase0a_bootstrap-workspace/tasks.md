## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-016 §1–2, SPEC-001 §2, SPEC-004 §3/§6/§7; DAG T0.1–T0.3
- [ ] 1.2 Create workspace + crates/veclite skeleton; pin rust-version (REL-002)
- [ ] 1.3 Add vectorizer-core = "3.5"; feature flags per API-050; workspace lints unwrap_used/expect_used = deny
- [ ] 1.4 Verify quantization/SIMD/compression compile on host and wasm32-unknown-unknown (CORE-004)
- [ ] 1.5 Port VecLiteError (all SPEC-004 §6 variants), Metric, Quantization, PayloadIndexKind
- [ ] 1.6 Implement CollectionOptions/OpenOptions builders with server-parity defaults
- [ ] 1.7 CI workflows: fmt + clippy -D warnings + test on 3 OS + wasm32 check + dependency deny-list (no network crates)

## 2. Testing
- [ ] 2.1 Unit tests pinning the defaults table (Cosine / 16 / 200 / 100 / SQ-8 / LZ4-1024)
- [ ] 2.2 Error display-string tests for every variant

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation (README dev section, CHANGELOG)
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass (fmt + clippy + cargo test green)
