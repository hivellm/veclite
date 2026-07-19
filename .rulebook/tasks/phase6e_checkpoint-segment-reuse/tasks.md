## 1. Implementation

Mechanism decided up front — see
[docs/analysis/checkpoint-segment-reuse/](../../../docs/analysis/checkpoint-segment-reuse/):
01 traces why the existing reuse gate never opens, 02 specifies the fix. The
failing test comes first, because "an idle checkpoint does not grow the file" is
the property nobody had written down.

- [ ] 1.1 Context: read both analysis files, then confirm against the code that the two reuse conditions still read as described (`clean_reuse` gated on `data.base`; `dirty` cleared only by `install_base`)
- [ ] 1.2 Regression test that fails today: file byte-length is unchanged across (a) N no-op checkpoints and (b) N open/close cycles with no writes, on an ordinary in-memory-tier collection
- [ ] 1.3 Add `sealed: Option<SealedRefs>` to `CollectionData` — `Vec<SegRef>` plus the `vector_count`/`tombstone_count` the TOC entry records. Independent of `base`, which stays exactly as it is
- [ ] 1.4 Load path: after `install_points`, install the TOC entry's `live_segments` as `sealed` and clear `dirty`. Do **not** change `apply_upsert` — a write genuinely does dirty the collection; the bug is that a load claims to be one
- [ ] 1.5 Commit path: return the per-collection refs `pager.checkpoint` already builds while writing, up through `commit` → `checkpoint_inner`, and install them as `sealed` with `dirty` cleared
- [ ] 1.6 `clean_reuse` falls back to `sealed` when `base` is absent; `base` keeps priority so the mmap tier is untouched
- [ ] 1.7 Drop `sealed` wherever `base` is dropped today (`collection.rs:1772`, vacuum's rebase). Reused refs are only valid within the same file — snapshot and vacuum write fresh files and already pass `allow_reuse = false`. Missing this points a TOC at pre-vacuum offsets, which is silent corruption and the worst outcome available here
- [ ] 1.8 Verify through built artefacts, not the source tree: rebuild the wheel, re-run the open/close and checkpoint sequences, confirm the sizes are flat

## 2. Testing

- [ ] 2.1 `cargo xtask crash` — the committed generation must stay self-consistent when some collections are reused and others freshly sealed. That mix exists today only for mmap collections; this change makes it the common case, so recovery is the real risk and a code review does not settle it
- [ ] 2.2 Explicit test that a vacuum invalidates `sealed`: vacuum, then checkpoint, then reopen and verify — catches the corruption 1.7 guards against
- [ ] 2.3 Conformance corpus green on Rust, Python and Node — reuse must not change any observable result
- [ ] 2.4 Cross-version file check: a file written by the fixed build opens on the published 0.1.1 build and vice versa. The format does not change, so this must hold

## 3. Documentation

- [ ] 3.1 SPEC-002: a checkpoint with no mutations MUST NOT grow the file
- [ ] 3.2 CHANGELOG, noting that existing oversized files need no migration — one `vacuum` reclaims them

## 4. Tail (docs + tests — check or waive with tailWaiver)

- [ ] 4.1 Update or create documentation covering the implementation
- [ ] 4.2 Write tests covering the new behavior
- [ ] 4.3 Run tests and confirm they pass
