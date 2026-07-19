# Fuzzing (SPEC-015 TST-050)

Coverage-guided libFuzzer targets over every parser that consumes untrusted
bytes. Target bodies live in `veclite::fuzz_api` (feature `fuzzing`) so the
same code backs three layers:

| Layer | Command | Runs on |
|---|---|---|
| Coverage-guided fuzzing | `cargo xtask fuzz [--seconds N] [--target <t>]` | Linux/macOS natively; Windows via the `rustlang/rust:nightly` container (automatic) |
| Committed-corpus regression | `cargo test -p hivellm-veclite --features fuzzing --test fuzz_regression` | stable, every platform — part of `cargo test --all-features` |
| Deterministic mutation sweeps | same test (`mutated_images_never_panic`) | stable, every platform |

## Targets

| Target | Parser | Requirement |
|---|---|---|
| `header` | 4 KiB file header (`Header::decode`) | SPEC-002 §2 |
| `toc` | MessagePack TOC (`Toc::decode`) | SPEC-002 §4 |
| `segment` | segment framing + decompression (`Segment::read`) | SPEC-002 §3 |
| `config` | CONFIG body — MessagePack options (`StoredConfig::decode`) | SPEC-002 §3.1 |
| `wal` | WAL replay scan (`Wal::scan`) | SPEC-003 |
| `filter` | portable filter documents (`Filter::from_json`) | SPEC-006 |
| `image` | whole-file open (`VecLite::deserialize`) | SPEC-015 "malformed file never crashes" |

## Corpus & accumulation

- `corpus/<target>/` — committed. Seeded with valid artifacts by
  `cargo xtask fuzz-seed`; coverage-guided runs grow it (commit the growth).
- `regressions/<target>/` — committed reproducers of fixed crashes; replayed
  forever by the stable regression test.
- `artifacts/<target>/` — uncommitted triage area for fresh crashes.
- `accumulation.log` — committed evidence trail toward the 72 h pre-1.0
  clean-accumulation gate (G6): one line per run, appended by
  `cargo xtask fuzz`.

A crash found by fuzzing is a release blocker: fix the parser, move the
reproducer into `regressions/<target>/`, and the normal gate guards it from
then on.
