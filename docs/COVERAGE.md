# Test coverage policy

VecLite's target is **100 % of *reachable* code**, measured honestly. Every test
asserts a real, documented behavior — no test exists solely to touch a line
("coverage theater"). The consequence is that the raw line-coverage number
plateaus below 100 %, on purpose: the uncovered remainder is code that *cannot*
run in a normal, portable test run, and forcing it to "covered" would mean
weakening the code or gaming the metric.

## Measuring

```bash
cargo xtask coverage                 # core + ffi, enforces the floor below
cargo llvm-cov -p veclite --tests --summary-only        # core detail
cargo llvm-cov -p veclite-ffi --tests --summary-only     # ffi detail
```

`--tests` is required: it aggregates the integration tests under `tests/` with
the unit tests. **Note the tooling quirk:** `cargo llvm-cov --show-missing-lines`
reports only the lib-unit-test run, so it lists lines that the integration tests
*do* cover. Trust the `--summary-only` per-file percentages, not the raw
`--show-missing-lines` list, when judging whether something is genuinely
uncovered.

Coverage counts the `#[cfg(test)]` modules and `tests/*.rs` files themselves, so
the denominator includes test code. The percentages below are therefore a floor
on the true *source* coverage, not a ceiling.

## Current floor (enforced by `cargo xtask coverage`)

| Crate | Line coverage |
|---|---|
| `veclite` (core) | ≥ 93 % |
| `veclite-ffi` (C ABI) | ≥ 95 % |

The Python and Node bindings are thin glue over the core; their behavior is
covered end-to-end by the conformance corpus (`cargo xtask conformance`, plus
`tests/conformance/runners/{python,node}`) and their own binding test suites
(`pytest crates/veclite-py`, `node --test crates/veclite-node/__test__`).

## The justified-unreachable residual

Every line still counted "uncovered" falls into one of these categories. None is
a real gap; each is documented here so a reviewer can distinguish "we forgot to
test this" (never acceptable) from "this cannot be tested honestly" (below).

1. **Invariant guards that only fire on a *broken* program state.**
   `debug_assert!(false, …)` and `unreachable!()` after an exhaustiveness check —
   e.g. `collection.rs` slot-below-base-count guard, the `nearest_slot`
   "expected at least one neighbour" arm. Reaching them would require the engine
   to already be corrupt in memory; a test that forces that state tests nothing
   real.

2. **Test-harness failure arms.** The repo convention for fallible calls in
   tests is `let Err(_) = … else { panic!(…) }` and
   `x.unwrap_or_else(|e| panic!("{e}"))`. The `panic!` branch executes only when
   a test *fails*, so a green run never covers it — yet the coverage tool counts
   those lines against the file. This is the single largest contributor to the
   residual (present in nearly every test module).

3. **OS-level I/O / mmap failure paths.** `storage/mmap.rs` map-failure branches
   and similar `io::Error` arms depend on the operating system refusing a
   syscall (a full disk, an unmappable region). These are not provocable
   deterministically or portably (Windows + Unix) from a unit test; they are
   exercised by the crash/fault-injection suite (`cargo xtask crash`), which runs
   out-of-process and so is not counted by in-process instrumentation.

4. **Architecture-gated SIMD stubs.** `simd/` keeps per-ISA function shapes
   stable for a future AVX2/NEON backend; today `dispatch::backend()` always
   resolves to the scalar backend, so alternate-ISA arms never execute on this
   CPU. Covering them requires the hardware backend that has not landed yet.

5. **API-shape branches unreachable through the public surface.** e.g. the FFI
   `VL_ERR_INTERNAL` catch-all (only a genuine panic inside `ffi()` reaches it)
   and the reopen-time "resolve a registered embedder" arm, which requires the
   embedder to be registered *before* `open()` returns — impossible when the
   only way to register is on an already-open handle.

## Rule for contributors

- Adding source code? Add tests that cover its **reachable** behavior in the
  same change. `cargo xtask coverage` must stay green.
- If a new line is genuinely unreachable, it must fit one of the categories
  above (or add a new, justified category here) — never lower the floor to
  accommodate an untested reachable path.
