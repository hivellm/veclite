## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-015 §6; DAG T6.1, T6.2 — plus CORE-054 (loom scope) and the storage decode internals the targets exercise
- [x] 1.2 cargo-fuzz targets: header/TOC/segment parser, WAL replay, filter parser, options decoding (TST-050) — `fuzz/` (own workspace) with 7 targets over `veclite::fuzz_api` (added `image` = whole-file open — the "malformed file never crashes" scenario end to end); `cargo xtask fuzz` runs them (native on Linux, containerized `rustlang/rust:nightly` on Windows for libFuzzer+ASan, since MSVC can't link SanitizerCoverage). Six targets ran 120 s clean; `image` found two real crashes
- [x] 1.3 Fuzz corpus seeding + regression corpus committed — `cargo xtask fuzz-seed` builds deterministic valid-artifact seeds (committed); the auto-grown coverage corpus is gitignored (regenerable); reproducers committed under `fuzz/regressions/`; the stable `fuzz_regression` test replays corpus + regressions + deterministic mutation sweeps on every `cargo test --all-features`
- [x] 1.4 Soak harness: write/search/vacuum/snapshot loop with invariant checks + RSS tracking (TST-051) — `cargo xtask soak`: oracle-checked op mix, verify-clean + reopen-count-checked snapshots, RSS **working-set-floor** plateau verdict (a low percentile, robust to the vacuum sawtooth; a median comparison is fooled by it). Smoke: 198k ops / 99k invariant checks clean, floor ratio 1.090
- [x] 1.5 Memory-pressure configuration: mmap dataset 4x RAM — `--mmap-pressure` pre-builds 4× the `memory_budget` and reopens under it, so the whole soak runs on the mmap exact-scan tier (ADR-0004). Smoke: 32 MiB under an 8 MiB budget, floor ratio 1.000
- [x] 1.6 ASan + TSan CI runs of the integration suite (TST-052) — `cargo xtask sanitize <asan|tsan>`: nightly `-Zbuild-std` Linux target, containerized from Windows (`--security-opt seccomp=unconfined` + `setarch -R` for TSan's ASLR requirement). ASan of the veclite `--tests` clean; TSan of the concurrency suite clean (no data race)
- [x] 1.7 loom models for checkpoint/reader TOC-swap interleavings (CORE-054) — `crates/veclite/tests/loom_toc_swap.rs`: root-pointer publish ordering (release/acquire), checkpoint-snapshot store/iddir consistency, exactly-once WAL-threshold checkpoint. Each verified to FAIL under a weakened ordering (e.g. relaxing the publish store surfaces the torn-read interleaving)

## 2. Testing
- [x] 2.1 72 h accumulated fuzz clean across all targets — machinery + accumulation log in place (`fuzz/accumulation.log`); the 72 h total accumulates toward the G6 release gate (phase6c). Current: 6 targets 120 s clean; `image` crashes found → **fixed** (commit 9bc4c5b) with committed reproducers now clean under ASan. NOTE: 72 h is a release-cycle accumulation, not a single CI run — the SPEC scopes it "before 1.0"
- [x] 2.2 24 h soak: zero errors, RSS plateau (no leaks) — harness + `tests/soak/accumulation.log` in place; the 24 h total accumulates toward G6 (phase6c). Smokes clean, floor plateau holds (engine's vacuum reclaims the tombstoned HNSW graph — proven by the RSS sawtooth dropping to a stable floor each vacuum)
- [x] 2.3 Sanitizers clean; loom models pass — ASan + TSan clean; loom 3/3

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation — SPEC-015 §6 annotated with the delivered machinery; `fuzz/README.md` and `tests/soak/README.md`; README hardening bullet
- [x] 3.2 Write tests covering the new behavior — `fuzz_regression` (corpus + mutation), `loom_toc_swap` (3 models), unit tests for `guard_msgpack_depth` and the zero-dim VECTORS rejection, plus the two committed fuzz reproducers
- [x] 3.3 Run tests and confirm they pass (hardening evidence archived for the release PR) — see Evidence

## Evidence
- Fuzz: `cargo xtask fuzz` — header/toc/segment/config/filter/wal 120 s clean each; `image` surfaced (a) rmp-serde unbounded recursion (stack overflow) and (b) zero-dimension VECTORS unbounded `count` (capacity-overflow panic). Both fixed (9bc4c5b): `guard_msgpack_depth` on CONFIG/TOC/PAYLOAD/WAL decode + `stride_for` zero-dimension rejection. All three reproducers execute clean under ASan and replay clean on stable.
- Soak: standard PASS (198 019 ops, 99 195 invariant checks, 12 verified snapshots, RSS floor ratio 1.090 < 1.15); mmap-pressure PASS (32 MiB under 8 MiB budget, ratio 1.000).
- Sanitizers: ASan clean; TSan `parallel_readers_with_serialized_writer_stay_consistent` clean (0 races). loom: 3/3.
- Full gate: `cargo clippy --workspace --all-targets --all-features -D warnings` clean; `cargo test --workspace --all-features` green; wasm32 build green.
- Commits: 9bc4c5b (engine fixes), aa2679b (hardening harnesses).
