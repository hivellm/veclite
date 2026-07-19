# Proposal: phase6e_checkpoint-segment-reuse

## Why

A `.veclite` file grows by a full copy of its segments every time the database is
checkpointed or closed, **even when nothing has been written**. Found by driving
the published CLI against a real database.

Opening and closing a database with zero writes, repeatedly:

```
inicial            17209
apos 1 open/close  30316
apos 2 open/close  43423
apos 3 open/close  56530
apos 4 open/close  69641
apos 5 open/close  82756
```

Explicit checkpoints behave the same — ~21.5 KB per no-op checkpoint on a
500-vector collection, perfectly linear:

```
 0 checkpoints ->  25642 bytes
 1 checkpoints ->  47182 bytes
 2 checkpoints ->  68724 bytes
 5 checkpoints -> 133368 bytes
10 checkpoints -> 241108 bytes
```

Reproduced identically through the Python binding and the Rust core, so this is
core behaviour, not a binding artefact.

Every application opens and closes the database once per run. A tool run daily
grows its file daily with no data changing; a long-running process that
checkpoints periodically for durability — the documented, encouraged pattern —
grows without bound while idle.

### Root cause

The design already has the right mechanism. `sealed_live_collections(inner,
allow_reuse = true)` asks each collection for `clean_reuse()`, which returns the
committed segment references when the collection has not been mutated, so the
checkpoint can reference them in place instead of resealing:

```rust
pub(crate) fn clean_reuse(&self) -> Option<(Vec<SegRef>, u64, u64)> {
    let data = self.inner.data.read();
    if data.dirty {
        return None;
    }
    data.base.as_ref().map(|b| (b.seg_refs.clone(), ...))
}
```

It is gated on `data.base`, which is installed only by `install_base` — and that
runs only on the **mmap tier**, when a collection is large enough to be mapped
rather than loaded into memory (`database.rs`, `if let Some(base) = loaded.base`).
An ordinary collection loads its points into memory, is marked dirty by that
load, has no `base`, and therefore never qualifies. The reuse path only ever
helps the mmap tier; every ordinary collection reseals everything on every
checkpoint.

Reopening the database does not help — verified, since a small collection still
takes the in-memory path.

### Mitigation that exists, and why it is not enough

`vacuum` reclaims the space (241,108 → 47,176 bytes in the run above). But
auto-vacuum triggers on the **tombstone ratio** (STG-072), and idle checkpoints
create no tombstones, so it never fires. The growth is only recoverable by an
explicit `vacuum`, which an operator has no reason to suspect is needed.

## What Changes

1. A collection that has not been mutated since it was last sealed reuses its
   committed segments on checkpoint, regardless of tier — not only when an mmap
   `base` is present. That means tracking "clean since last seal" independently
   of the mmap base, and clearing it when a checkpoint installs the sealed state.
2. Loading a collection's points into memory must not by itself mark it dirty:
   the state on disk and the state in memory are identical at that moment.
3. A regression test that asserts the file size is unchanged across repeated
   no-op checkpoints and open/close cycles — the property that was never
   asserted, which is why this went unnoticed.

## Impact

- Affected specs: SPEC-002 (STG-070 checkpoint, STG-071/072 vacuum), SPEC-003
  (checkpoint semantics)
- Affected code: `crates/veclite/src/collection.rs` (`clean_reuse`, the `dirty`
  flag and its transitions), `crates/veclite/src/database.rs`
  (`sealed_live_collections`, `checkpoint_inner`, the load path)
- Breaking change: NO — file format and API unchanged; files written by the
  current build stay readable, they are merely larger than they need to be
- User benefit: a database that is opened and closed, or checkpointed, without
  writes stops growing. Removes an unbounded growth path that today is only
  recoverable by an explicit `vacuum` an operator has no reason to run.

## Out of scope

The CLI was exercised in the same run and behaved correctly throughout:
`inspect` reported segments accurately, `verify` returned exit 0 on a clean file
and exit 1 on a file with one flipped byte — naming the offset, segment type and
collection — `vacuum` reclaimed the space, and `snapshot` produced a 4x smaller
standalone copy that opened with both collections, working search and a resolved
alias. No defects found there.
