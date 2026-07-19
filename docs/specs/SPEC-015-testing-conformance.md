# SPEC-015 — Testing, Conformance & Benchmarks

| | |
|---|---|
| **Status** | Draft |
| **Phase / tasks** | Continuous · T1.6, T1.7, T2.10, T4.5, T5.6, T6.1, T6.2, T6.4 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-65, FR-73; NFR-01–05, NFR-10 |
| **Planning source** | [04-storage §test plan](../vectorizer-lite/04-storage-format.md), [06-sdk §conformance](../vectorizer-lite/06-sdk-bindings.md), [08-roadmap](../vectorizer-lite/08-roadmap.md) |

Requirement IDs `TST-xxx`. Repository layout: `tests/crash/`, `tests/compat/`, `tests/conformance/`, `tests/bench/`.

## 1. Test taxonomy & gates

| Layer | Tooling | Blocking gate |
|---|---|---|
| Unit + property tests | `cargo test`, `proptest` | every PR |
| Crash-safety suites | custom harness + fault-injection VFS | G2, then nightly |
| Server-parity / shared conformance | corpus in `tests/compat/` run in **both** repos | G1, G3, G5 |
| Binding conformance | YAML corpus + per-language runners | G4, G5; release-blocking per binding |
| Benchmarks | criterion + reference profiles | G1 (targets), G6 (report) |
| Fuzzing | cargo-fuzz | G6 (72 h clean) |
| Soak / sanitizers | 24 h loop, ASan/TSan, loom (targeted) | G6 |

- **TST-001** Quality gates from the roadmap apply to every phase: fmt, clippy `-D warnings` (incl. `unwrap_used`/`expect_used` deny), tests green on Linux/macOS/Windows before a phase exits.

## 2. Crash-safety suites (normative for G2)

- **TST-010** **kill-9 harness**: a driver process runs a randomized write workload (upsert/delete/create/drop/alias/vocab mix, all three durability modes) while a supervisor kills it at random points; reopen and assert: file opens, `verify` clean, all acked-`Full` commits present, model-state equivalence for replayed WAL. 10 000 iterations, zero corruption (NFR-05).
- **TST-011** **Torn-write fuzzing**: a fault-injection VFS shim truncates/reorders the tail after every N bytes written (sweep N); assert SPEC-002 STG-003 and SPEC-003 WAL-011 invariants.
- **TST-012** **Bit-flip drills**: flip random bits per segment type and in the WAL; open fails with `Corrupt` naming the segment (or recovers, for WAL tail) — never UB, never a silently wrong answer. `read_only` open past a damaged uncommitted tail succeeds.
- **TST-013** The full crash suite runs on Windows CI too (mmap/truncate risk area — PRD risk table).

## 3. Binding conformance corpus (FR-65)

- **TST-020** One YAML corpus (`tests/conformance/*.yaml`) describing operations and expected outcomes, executed by runners in Rust, Python, Node, Go, C#, and WASM. A binding is **release-blocked** until the corpus passes on its full platform matrix.
- **TST-021** Corpus coverage (minimum): defaults table (SPEC-004 §3); every error variant and its code/message; CRUD + scroll semantics; filter semantics table (SPEC-006); hybrid rankings (SPEC-007); auto-embed reopen determinism (SPEC-005); chunker boundaries; in-memory ≡ file-backed behavior.
- **TST-022** Score comparisons use tolerance **1e-5**; orderings and id sets are exact. Every case carries a stable id (`conf-xxx`) referenced in failure output.
- **TST-023** The corpus is versioned with the format: new cases may be added freely; changing an expected value requires the same review bar as a format change (it means behavior changed).

## 4. Server-parity & interop testing

- **TST-030** **Parity harness** (G1): standard benchmark corpus (pinned dataset + queries, committed or fetched-by-hash) loaded into VecLite and a pinned Vectorizer server version; top-10 overlap ≥ 0.99 (NFR-04); identical defaults asserted.
- **TST-031** **Shared conformance corpus** (G3/G5): filter + hybrid golden results generated once, reviewed, then enforced in both repos' CI (IOP-002). Divergence fails the build in whichever repo changed.
- **TST-032** **Graduation round-trip** (G5): SPEC-013 §4 criteria automated against a dockerized server in CI (allowed to be a scheduled job if runtime > PR budget).

## 5. Benchmarks

- **TST-040** Criterion benches (in `tests/bench/`) with pinned reference profiles (resolves OQ-1 at T1.6; record CPU model, RAM, OS in the report):
  | Bench | Target | Requirement |
  |---|---|---|
  | search p50, 1 M × 512-dim, SQ-8, warm | < 3 ms | NFR-01 |
  | warm mmap open, same file | < 100 ms | NFR-02 |
  | index build vs server single-node | ≤ 2× | NFR-03 |
  | batch insert throughput | tracked (no gate) | — |
  | filtered search (selective/broad) | tracked (no gate) | — |
- **TST-041** CI runs a scaled-down smoke bench per PR (regression fence at ±20 %); full-size benches run nightly on the reference runner.
- **TST-042** **1.0 benchmark report** (T6.4): reproducible harness comparing VecLite vs sqlite-vec, LanceDB embedded, Chroma embedded, and Vectorizer server — datasets, code, and hardware disclosed; results published with the docs site. Honesty rule: publish losses too.

## 6. Fuzzing, soak, sanitizers (G6)

Machinery implemented in phase6a; the 72 h / 24 h figures are **accumulated
across runs** (evidence logs committed) and gate the 1.0 release (phase6c).

- **TST-050** cargo-fuzz targets: file/header/TOC/segment parser, WAL replay, filter document parser, MessagePack option decoding. 72 h accumulated clean before 1.0; corpus committed for regression. — `fuzz/` (7 targets over `veclite::fuzz_api`), `cargo xtask fuzz-seed` / `fuzz` (native on Linux, containerized on Windows), committed corpus replayed on stable by the `fuzz_regression` test in the normal gate; accumulation trail in `fuzz/accumulation.log`.
- **TST-051** 24 h soak: continuous write/search/vacuum/snapshot loop with invariant checks; mmap dataset 4× RAM (memory-pressure path); zero leaks (RSS plateau), zero errors. — `cargo xtask soak [--minutes N] [--mmap-pressure] [--budget-mb M]`: oracle-checked operation mix, verify-clean snapshots, RSS plateau verdict (first/last-quartile medians, 1.15× limit); the pressure mode builds 4× the `memory_budget` and runs on the mmap exact-scan tier; trail in `tests/soak/accumulation.log`.
- **TST-052** Sanitizers: ASan + TSan runs of the integration suite; targeted `loom` models for the checkpoint/reader TOC-swap interleavings (CORE-054). — `cargo xtask sanitize <asan|tsan>` (nightly `-Zbuild-std`, Linux target; containerized on Windows; `onnx` excluded — prebuilt foreign runtime); loom models in `crates/veclite/tests/loom_toc_swap.rs` (root-pointer publish ordering, checkpoint-snapshot consistency, exactly-once threshold checkpoint — each verified sensitive to a weakened ordering).

## 7. Acceptance criteria

This spec is itself the acceptance machinery; its own completeness checks:

1. Every FR/NFR in the [PRD](../PRD.md) maps to at least one suite above (traceability table maintained in `tests/README.md`).
2. Gate definitions in the [DAG](../DAG.md) reference only suites defined here.
3. All suites runnable locally with one command each (`cargo xtask crash`, `cargo xtask conformance`, …) — CI-only tests are not accepted.
