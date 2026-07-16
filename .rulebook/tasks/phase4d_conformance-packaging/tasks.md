## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-015 §3, SPEC-016 §2–3; DAG T4.5, T4.6 and gate G4
- [x] 1.2 Corpus schema + case-id convention (conf-xxx); author coverage per TST-021 — tests/conformance/README.md + corpus/*.yaml (34 cases, 7 suites)
- [x] 1.3 Rust reference runner (defines golden outcomes) — `cargo xtask conformance [--bless]`, golden.json sidecar
- [x] 1.4 Python + Node runners consuming the same YAML; 1e-5 score tolerance (TST-022) — runners/{python/run.py,node/run.mjs}; all 34 cases green on Rust+Py+Node. Completed binding surface: sparse-lane upsert (py+node), scroll+chunk (py), refit+chunk+aliases (node)
- [x] 1.5 Wheel matrix CI (maturin) + prebuild matrix CI (napi-rs) per FR-66 — .github/workflows/veclite-packaging.yml (full FR-66 matrix; conformance runs on each native artifact)
- [x] 1.6 Clean-machine install jobs: no-toolchain containers run pip/npm install + quickstart (REL-020) — veclite-clean-install.yml (python:3.12-slim / node:20-slim, asserts no cargo, runs examples/quickstart.{py,mjs})
- [x] 1.7 Release workflow skeleton with atomic all-or-nothing publish (REL-012) — veclite-release.yml (gate job needs every build+conformance leg before any publish)
- [x] 1.8 Dry-run publish to TestPyPI and npm dist-tag next — veclite-release.yml workflow_dispatch target=test → TestPyPI + npm --tag next

## 2. Testing
- [ ] 2.1 Corpus green on Rust, Python, Node across the full platform matrix
- [ ] 2.2 Clean-machine quickstarts pass for pip and npm (gate G4 criterion)
- [ ] 2.3 Corpus mutation guard: changing an expected value requires review (documented in tests/conformance/README)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass (gate G4 evidence attached)
