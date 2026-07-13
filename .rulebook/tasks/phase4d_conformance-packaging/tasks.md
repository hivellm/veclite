## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-015 §3, SPEC-016 §2–3; DAG T4.5, T4.6 and gate G4
- [ ] 1.2 Corpus schema + case-id convention (conf-xxx); author coverage per TST-021
- [ ] 1.3 Rust reference runner (defines golden outcomes)
- [ ] 1.4 Python + Node runners consuming the same YAML; 1e-5 score tolerance (TST-022)
- [ ] 1.5 Wheel matrix CI (maturin) + prebuild matrix CI (napi-rs) per FR-66
- [ ] 1.6 Clean-machine install jobs: no-toolchain containers run pip/npm install + quickstart (REL-020)
- [ ] 1.7 Release workflow skeleton with atomic all-or-nothing publish (REL-012)
- [ ] 1.8 Dry-run publish to TestPyPI and npm dist-tag next

## 2. Testing
- [ ] 2.1 Corpus green on Rust, Python, Node across the full platform matrix
- [ ] 2.2 Clean-machine quickstarts pass for pip and npm (gate G4 criterion)
- [ ] 2.3 Corpus mutation guard: changing an expected value requires review (documented in tests/conformance/README)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass (gate G4 evidence attached)
