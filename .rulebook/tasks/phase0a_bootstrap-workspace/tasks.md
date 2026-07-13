## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-016 §1–2, SPEC-001 §2, SPEC-004 §3/§6/§7; DAG T0.1–T0.3
- [x] 1.2 Create workspace + crates/veclite skeleton; pin rust-version (REL-002) <!-- provisional MSRV 1.85: upstream declares none; floor imposed by edition 2024 -->
- [ ] 1.3 Add vectorizer-core = "3.5"; feature flags per API-050; workspace lints unwrap_used/expect_used = deny <!-- BLOCKED (partial): feature flags + lints DONE; the vectorizer-core dependency itself cannot be added — crate is unpublished on crates.io AND its mandatory deps (axum, tonic, rmcp, umicp-core http2/websocket) violate NFR-08. Needs upstream change in hivellm/vectorizer: feature-gate the wire-error mappings so default-features=false is lean, then publish (or git-dep a pinned rev). Decision escalated to the user. -->
- [ ] 1.4 Verify quantization/SIMD/compression compile on host and wasm32-unknown-unknown (CORE-004) <!-- BLOCKED on 1.3 for the vectorizer-core parts; the veclite skeleton itself builds green on host + wasm32 (verified locally and in CI job wasm32-build) -->
- [x] 1.5 Port VecLiteError (all SPEC-004 §6 variants), Metric, Quantization, PayloadIndexKind <!-- tested -->
- [x] 1.6 Implement CollectionOptions/OpenOptions builders with server-parity defaults <!-- tested -->
- [x] 1.7 CI workflows: fmt + clippy -D warnings + test on 3 OS + wasm32 check + dependency deny-list (no network crates) <!-- rust-lint/rust-test cover fmt+clippy+3-OS; veclite-checks.yml adds wasm32 + deny-list + MSRV -->

## 2. Testing
- [x] 2.1 Unit tests pinning the defaults table (Cosine / 16 / 200 / 100 / SQ-8 / LZ4-1024)
- [x] 2.2 Error display-string tests for every variant

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (README dev section, CHANGELOG)
- [x] 3.2 Write tests covering the new behavior <!-- 7 unit tests + 1 doctest -->
- [x] 3.3 Run tests and confirm they pass (fmt + clippy + cargo test green) <!-- clippy -D warnings clean; 7+1 tests pass; wasm32 build green -->
