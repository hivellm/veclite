# Proposal: phase4d_conformance-packaging

## Why
DAG T4.5 + T4.6 close gate G4: one YAML corpus executed by every binding guarantees behavioral parity, and the packaging CI proves the no-toolchain bar — pip/npm install on clean machines with no Rust toolchain (FR-65, FR-66).

## What Changes
- Conformance YAML corpus in tests/conformance/: defaults, every error variant, CRUD/scroll, filters, hybrid rankings, auto-embed reopen, chunker, memory==file (TST-020..023)
- Runners for Rust, Python, Node with stable case ids and 1e-5 score tolerance
- Packaging CI: maturin wheel matrix + napi prebuild matrix per FR-66 (manylinux/musllinux/macOS/Windows x x64/arm64)
- Clean-machine install jobs: fresh containers/VMs run pip install + npm install + quickstart (REL-020)
- Release dry-run to TestPyPI + npm dist-tag next (SPEC-016 acceptance 2)

## Impact
- Affected specs: SPEC-015 §3, SPEC-016 §2–3
- Affected code: tests/conformance/, .github/workflows/release.yml
- Breaking change: NO
- User benefit: gate G4 — install-and-run proven on every supported platform before widening the ecosystem
