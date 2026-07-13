# Proposal: phase6c_release-1-0

## Why
DAG T6.5 closes gate G6 — the 1.0.0 release itself: format stability pledge, SemVer policy, and the atomic multi-channel publish across crates.io, PyPI, npm, Go, NuGet, and GitHub releases (PRD §9 go/no-go, NFR-11, NFR-12).

## What Changes
- Format stability pledge published: every future 1.x reads v1 files, backed by the golden-file corpus (REL-031)
- SemVer + lockstep versioning policy doc; FFI additive-only rule documented (REL-030/032)
- .github/RELEASE_TEMPLATE.md with the PRD §9 checklist item-by-item
- Release dry-run at test indexes, then tag → atomic all-or-nothing publish of the full artifact matrix (REL-012) → post-publish smoke installs
- PRD §9 checklist verified: gates G0–G5 green, crash suite within 7 days, conformance matrix green, graduation overlap, fuzz/soak/sanitizers clean, footprint budgets, clean-machine installs

## Impact
- Affected specs: SPEC-016 §4/§6
- Affected code: release workflow finalization, policy docs, version stamping
- Breaking change: NO
- User benefit: VecLite 1.0.0 available in every ecosystem simultaneously, with enforceable stability promises
