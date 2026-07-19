# Proposal: phase5d_vecdb-interop-cli

## Why
DAG T5.5 + T5.6 close gate G5: the graduation path is VecLite's strategic funnel thesis — data must move both ways between .veclite and the server's .vecdb with >= 0.99 result overlap, and the CLI makes it operable (FR-70–73).

## What Changes
- vecdb-interop feature: export to server Compact layout (.vecdb + .vecidx) with lossless quantized blocks (IOP-001, IOP-010..013)
- Import of both server layouts via detect_format (Compact + Legacy); degradation matrix with warnings, encrypted-payload refusal, origin_provider recording (IOP-020..023)
- veclite-cli crate (binary name veclite, resolves OQ-4): inspect/export/import/vacuum/snapshot/verify with stable exit codes, no network (CLI-001..004)
- Graduation round-trip test vs dockerized server; shared conformance corpus wired into both repos (IOP-002, TST-032)

## Impact
- Affected specs: SPEC-013, SPEC-014 (all)
- Affected code: crates/veclite/src/interop/ (feature-gated), crates/veclite-cli/ (new)
- Breaking change: NO
- User benefit: start embedded, graduate to the server (or take a slice offline) with data and scoring intact
