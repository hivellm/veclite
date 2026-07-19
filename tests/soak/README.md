# Soak & memory-pressure evidence (SPEC-015 TST-051)

`cargo xtask soak [--minutes N] [--mmap-pressure] [--budget-mb M]` runs the
sustained-operation harness: a continuous write / search / vacuum / snapshot
mix against a file-backed database with an in-memory oracle.

- **Invariants per cycle**: sampled `get` matches the oracle (vectors within
  1e-5 after cosine normalization, payloads exact); `len` matches; searches
  and scrolls return only live ids with finite scores; every periodic
  snapshot passes the full `verify` integrity pass and reopens with matching
  counts.
- **Leak verdict**: RSS is sampled every 10 s, counting only steady-state
  samples (live set at the `--live-cap`, default 20 000 — while the set is
  still growing, RSS growth is the data, not a leak). After a warm-up
  quarter, the last-quartile median must stay within 1.15× of the
  first-quartile median — a monotonic-growth trend fails the run. Short
  smoke runs lower `--live-cap` so they reach steady state at all.
- **`--mmap-pressure`**: pre-builds a dense dataset 4× the configured
  `memory_budget` and reopens under that budget, so the whole run executes on
  the mmap exact-scan tier (ADR-0004) — "dataset 4× RAM" realized through the
  budget knob, which is the enforced memory ceiling.

`accumulation.log` (committed) is the evidence trail toward the 24 h pre-1.0
gate (G6): one line per run, appended by the harness. The 24 h and 72 h
targets are *accumulated across runs* before the 1.0 release cycle
(phase6c); any failing run is a release blocker.
