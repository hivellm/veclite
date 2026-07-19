# Proposed fix: record the committed refs at the two points where they are known

The analysis in [01](01-why-idle-checkpoints-grow-the-file.md) ends with the
observation that both moments where a collection is provably in sync with the
file already know its committed segment references, and neither records them.
That is the whole fix.

## Shape

Give the collection a place to hold the refs of the segments it is currently in
sync with, independent of the mmap base:

```rust
struct CollectionData {
    // ...
    base: Option<BaseTier>,      // mmap tier only — unchanged
    sealed: Option<SealedRefs>,  // committed segments this state matches
    dirty: bool,
}
```

`SealedRefs` carries what `clean_reuse` must return: `Vec<SegRef>` plus the
`vector_count` and `tombstone_count` recorded in the TOC entry.

Populate it at the two points:

**On load.** The TOC entry the loader already read carries `live_segments` — the
committed refs for that collection. After `install_points`, install them and
clear `dirty`. This is the honest statement that memory and disk agree, which
they do.

**On commit.** `pager.checkpoint` already builds the `refs` per collection as it
writes. Return them from `checkpoint` → `commit` → `checkpoint_inner`, and hand
each collection its own set: install as `sealed`, clear `dirty`.

Then widen the gate:

```rust
pub(crate) fn clean_reuse(&self) -> Option<(Vec<SegRef>, u64, u64)> {
    let data = self.inner.data.read();
    if data.dirty {
        return None;
    }
    data.base
        .as_ref()
        .map(|b| (b.seg_refs.clone(), b.vector_count, b.tombstone_count))
        .or_else(|| data.sealed.as_ref().map(|s| (s.refs.clone(), s.vector_count, s.tombstone_count)))
}
```

## What deliberately does not change

**`apply_upsert` keeps setting `dirty = true`.** It is right: a write does dirty
the collection. The bug is not that writes dirty it, but that a load claims to be
a write and that a commit never un-dirties. Changing `apply_upsert` to take a
"this is a load" flag would spread the concern across every write path; setting
the state once, after the load completes, keeps it in one place.

**The mmap tier keeps its own path.** `base` is consulted first and behaves
exactly as today. The `sealed` fallback only engages where there is no base —
i.e. precisely the case that is broken now. This matters because the mmap path is
the one that works, and it must not regress.

**The file format does not change.** Reuse only affects which offsets a new TOC
entry points at; the segments, the TOC and the header are untouched. Files stay
readable both ways.

## Where this can go wrong

Two invariants have to hold, and both are worth an explicit test rather than an
argument.

**Reused refs must still be live in the same file.** `sealed_live_collections`
already encodes this: `allow_reuse` is `true` only for checkpoint, and `false`
for snapshot and vacuum, which write fresh files where every offset is
invalidated (`database.rs:277`). A `sealed` set must therefore be dropped on
vacuum, exactly as `base` is dropped today at `collection.rs:1772`. Missing that
would produce a TOC pointing at offsets from the pre-vacuum file — silent
corruption, and the most dangerous outcome of this change.

**Recovery must be unaffected.** The committed generation must stay
self-consistent when some collections are reused and others are freshly sealed —
a mix that already occurs today for mmap collections, but will become the common
case. `cargo xtask crash` is the gate, not a code review.

## Expected result

The regression test is a size assertion, which is the property nobody had
written down:

- a database opened and closed N times with no writes has the same byte length
  as after the first close;
- N no-op checkpoints leave the size unchanged after the first.

Existing oversized files need no migration: a single `vacuum` reclaims them.
