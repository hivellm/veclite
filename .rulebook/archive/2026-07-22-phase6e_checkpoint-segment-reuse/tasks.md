## 1. Implementation

Mechanism decided up front — see
[docs/analysis/checkpoint-segment-reuse/](../../../docs/analysis/checkpoint-segment-reuse/):
01 traces why the existing reuse gate never opens, 02 specifies the fix. The
failing test comes first, because "an idle checkpoint does not grow the file" is
the property nobody had written down.

- [x] 1.1 Context: confirmed both reuse conditions read as the analysis describes — `clean_reuse` gated on `data.base`, `dirty` cleared only by `install_base`, and `apply_upsert` dirtying on every point the load installs
- [x] 1.2 Regression test written first and confirmed failing: 25,642 -> 133,368 bytes over five idle checkpoints. Covers both no-op checkpoints and open/close cycles
- [x] 1.3 `sealed: Option<SealedRefs>` added to `CollectionData`, independent of `base`
- [x] 1.4 Load path records the TOC entry's `live_segments` after `install_points` and clears `dirty`. `apply_upsert` untouched, as planned — a write does dirty the collection
- [x] 1.5 Commit path: no signature change needed after all — `pager.checkpoint` already returns the `Toc` and every `CollEntry` carries `coll_id` + `live_segments`; `commit` was discarding it. Threading it out was the whole change
- [x] 1.6 `clean_reuse` falls back to `sealed` with `base` keeping priority; mmap_tier suite green (6/6, incl. `clean_checkpoint_carries_forward_and_reopens`)
- [x] 1.7 `sealed` dropped at the vacuum rebase and in `drop_base_unchecked`. Note: `dirty = true` at the rebase already blocks reuse, so this is defence in depth — verified by removing it and watching the test still pass
- [x] 1.8 Verified through built artefacts: rebuilt wheel and linux-x64 addon both show a flat file across idle checkpoints and open/close cycles

## 2. Testing

- [x] 2.1 `cargo xtask crash` — run and green (in-process suite 5/5 at 10k iters; kill-9 harness 500/500). The first full run FAILED at kill iter 102 with `header: bad magic` — root-caused as a PRE-EXISTING window in `Pager::create` (gen-0 TOC written before the header; no previous chain to fall back on), not a reuse regression: the same failure hit the nightly CI on main (commit cb6545d, before this change) at iter 0 on Windows. Fixed separately as SPEC-002 STG-053 (creation via sibling temp + atomic rename); after that fix the harness is 500/500 clean with carry-forward as the common case
- [x] 2.2 Vacuum-then-checkpoint-then-reopen test added. It verifies the round-trip to the exact surviving point set; it does NOT isolate the sealed guard (see 1.7) and its comment says so rather than overclaiming
- [x] 2.3 Conformance 34/34 on Python and Node against rebuilt artefacts
- [x] 2.4 Cross-version verified both ways: a file from this build passes the published 0.1.1 CLI `verify` (exit 0); a file written by published 0.1.1 opens here with all 50 points and working search. Same data now 9,520 -> 6,811 bytes

## 3. Documentation

- [x] 3.1 SPEC-002 STG-052 added: a checkpoint must not rewrite a collection already matching the file, and one with nothing to persist must leave the byte-length unchanged
- [x] 3.2 CHANGELOG entry under Fixed, noting no migration is needed — one `vacuum` reclaims existing oversized files

## 4. Tail (docs + tests — check or waive with tailWaiver)

- [x] 4.1 Update or create documentation covering the implementation — SPEC-002 STG-052 + CHANGELOG (3.1/3.2), plus the implementation postmortem appended to docs/analysis/checkpoint-segment-reuse/02-proposed-fix.md; spec citations in code comments corrected from STG-070 (snapshot) to STG-052
- [x] 4.2 Write tests covering the new behavior — covered by 1.2/2.2 plus the mixed carry-forward/reseal commit test in tests/persistence.rs
- [x] 4.3 Run tests and confirm they pass — full gate green: cargo check, clippy -D warnings, fmt, full workspace suite, and `cargo xtask crash` (see 2.1)
