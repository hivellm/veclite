# Proposal: phase1d_bench-parity-gate

## Why
DAG T1.6 + T1.7 close gate G1: without the criterion benches and the server-parity harness there is no objective proof the extracted engine matches Vectorizer quality (NFR-01..04). This cycle also resolves PRD OQ-1 by pinning the reference hardware profile.

## What Changes
- Criterion benches in tests/bench/: search p50 (1M x 512-dim SQ-8 target < 3 ms), index build time vs server, batch insert throughput (TST-040)
- Parity harness in tests/compat/: same pinned corpus into VecLite and a pinned Vectorizer server; top-10 overlap >= 0.99 (TST-030)
- CI smoke bench per PR with +-20% regression fence; full bench nightly (TST-041)
- Reference hardware profile documented (resolves OQ-1); PRD updated

## Impact
- Affected specs: SPEC-015 §4–5; PRD §12 (OQ-1 resolved)
- Affected code: tests/bench/, tests/compat/, CI workflows
- Breaking change: NO
- User benefit: gate G1 green means the engine provably matches the server before persistence work begins
