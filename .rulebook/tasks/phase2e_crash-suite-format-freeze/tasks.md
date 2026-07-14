## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-015 §2, SPEC-002 §9; DAG T2.10 and gate G2
- [x] 1.2 File-state fault injection: torn-WAL-tail + torn-main-tail + WAL/main bit-flip sweeps assert the STG-003 / WAL-011 invariants (crash_safety.rs)
- [x] 1.3 kill-9 harness: randomized workload + subprocess supervisor across all three durability modes (xtask crash + crash_and_reopen_reconstructs_model_all_durability_modes, TST-010)
- [x] 1.4 Bit-flip drills per segment (gates::single_bit_flip_in_segment) and WAL (crash_safety::bit_flip_in_wal / bit_flip_in_committed_file); Corrupt naming or valid-prefix recovery (TST-012)
- [x] 1.5 Model-state equivalence checker (oracle model in crash_safety.rs + xtask acked-id oracle)
- [x] 1.6 Nightly CI on Linux/macOS/Windows (veclite-crash.yml, TST-013); one-command runner (cargo xtask crash)
- [x] 1.7 10 000-iteration suite: parameterized (VECLITE_CRASH_ITERS), run 10 000 nightly in CI; local sample 1 500 in-process + 200 kill-9, zero corruption
- [x] 1.8 Freeze: SPEC-002/003 marked frozen-normative, v1 golden committed to tests/compat/golden/, golden check wired into CI

## 2. Testing
- [x] 2.1 Suite passes zero-corruption at scale (nightly 10 000; local 1 500 + 200 kill-9 clean) (NFR-05)
- [x] 2.2 All acked-Full commits present after every kill point (xtask verify_after_kill)
- [x] 2.3 Golden-file compatibility check green (tests/golden.rs + veclite-crash.yml golden-compat)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (SPEC-002/003 freeze status, docs/specs/README.md, CHANGELOG, README, SPEC-003 §3 crc note)
- [x] 3.2 Write tests covering the new behavior (crash_safety.rs (5), golden.rs (2), iddir OOM regression, WAL-header-crc coverage)
- [x] 3.3 Run tests and confirm they pass (all suites green; clippy clean; wasm32 builds; MSRV 1.87 builds)

<!-- Two robustness bugs surfaced by this gate were fixed before the freeze:
     the WAL entry crc now covers the header fields (was body-only, a corrupt
     coll_id was silently misrouted), and IdDir::decode bounds its allocation by
     the input length (an adversarial bucket_count could OOM-abort the decode
     fuzz). The MSRV was raised to 1.87 (hnsw_rs =0.3.4 uses is_multiple_of). -->
