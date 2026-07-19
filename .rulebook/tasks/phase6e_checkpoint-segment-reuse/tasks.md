## 1. Implementation

The failing test comes first: the property "a no-op checkpoint does not grow the
file" was never asserted, which is exactly why this shipped.

- [ ] 1.1 Context: read the proposal, SPEC-002 STG-070/071/072, and trace `dirty` through `collection.rs` — every site that sets it, and `install_base` as the only site that clears it
- [ ] 1.2 Regression test that fails today: file size is unchanged across (a) repeated no-op checkpoints and (b) repeated open/close cycles with no writes, for an ordinary in-memory-tier collection
- [ ] 1.3 Decide the mechanism before writing it: `clean_reuse` needs to work off "clean since last seal" rather than off the presence of an mmap `base`. Establish where the sealed segment refs live for a non-mmap collection, since today only `BaseTier` carries them
- [ ] 1.4 Loading a collection's points must not mark it dirty — at that instant memory and disk agree. Check each `dirty = true` site against that claim rather than flipping the load path alone
- [ ] 1.5 A checkpoint that seals a collection records the resulting segment refs and marks it clean, so the *next* checkpoint can reuse them
- [ ] 1.6 Confirm the mmap tier still reuses as before — it is the one path that works today, and it must not regress
- [ ] 1.7 Verify through built artefacts, not just the source tree: rebuild the wheel, re-run the open/close and checkpoint sequences, and confirm the numbers are flat

## 2. Testing

- [ ] 2.1 Crash safety is the risk here: reusing segments across a checkpoint changes what the committed generation references. Run `cargo xtask crash` and confirm recovery is unaffected
- [ ] 2.2 Conformance corpus green on Rust, Python and Node — reuse must not change any observable result
- [ ] 2.3 A file written by the fixed build opens on the current published build and vice versa (the format does not change, so this must hold)

## 3. Documentation

- [ ] 3.1 SPEC-002: state that a checkpoint with no mutations MUST NOT grow the file — the property this task adds
- [ ] 3.2 CHANGELOG entry, noting that existing oversized files are reclaimed by a single `vacuum`

## 4. Tail (docs + tests — check or waive with tailWaiver)

- [ ] 4.1 Update or create documentation covering the implementation
- [ ] 4.2 Write tests covering the new behavior
- [ ] 4.3 Run tests and confirm they pass
