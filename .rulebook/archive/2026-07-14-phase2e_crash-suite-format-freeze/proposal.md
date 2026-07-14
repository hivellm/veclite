# Proposal: phase2e_crash-suite-format-freeze

## Why
DAG T2.10 closes gate G2: the crash suite is the proof behind NFR-05 (10 000 iterations, zero corruption) and its pass freezes format v1 permanently — every future 1.x must read files written from this point on (NFR-11). Nothing in phases 3+ may start before this gate.

## What Changes
- kill-9 harness: driver process under randomized write workload, supervisor kills at random points, reopen asserts invariants (TST-010)
- Fault-injection VFS shim for torn-write fuzzing (TST-011)
- Bit-flip drills per segment type and WAL (TST-012)
- Suite wired into nightly CI on Linux/macOS/Windows (TST-013)
- Format freeze: SPEC-002 marked frozen-normative; v1 golden files committed to tests/compat/golden/ and verified every CI run

## Impact
- Affected specs: SPEC-015 §2, SPEC-002 (status → frozen), docs/specs/README.md freeze note
- Affected code: tests/crash/, storage fault-injection hooks, CI nightly workflow
- Breaking change: NO (it prevents future breaking changes)
- User benefit: the durability promise is proven, not claimed; format stability pledge becomes enforceable
