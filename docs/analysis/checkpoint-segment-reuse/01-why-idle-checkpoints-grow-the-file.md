# Why an idle checkpoint grows the file by a full copy

## The observation

Opening and closing a database with **zero writes** adds a full copy of its
segments each time:

```
initial            17209
after 1 open/close 30316
after 2 open/close 43423
after 3 open/close 56530
after 4 open/close 69641
after 5 open/close 82756
```

Explicit checkpoints behave the same: ~21.5 KB per no-op checkpoint on a
500-vector collection, perfectly linear (25,642 → 241,108 bytes over ten). The
Python binding and the Rust core produce byte-identical numbers, so this is core
behaviour.

## The commit path

`Database::checkpoint` → `checkpoint_inner` → `persistence.commit(
sealed_live_collections(inner, allow_reuse = true))`.

`sealed_live_collections` has exactly two branches per collection
(`database.rs:297`):

```rust
if allow_reuse && let Some((refs, vector_count, tombstone_count)) = handle.clean_reuse() {
    colls.push(CheckpointColl { /* ... */ segments: Vec::new(), reused: Some(refs) });
    continue;
}
// otherwise: seal::seal(...) — re-encode every segment from live points
```

The reuse branch is the whole point: the pager honours it by writing nothing at
all and pointing the new TOC entry at the existing bytes (`pager.rs:204`):

```rust
if let Some(refs) = c.reused {
    // Carry-forward: the committed segments are immutable and still
    // live in this same file — reference them, write nothing.
```

So the machinery to not grow the file already exists and works. The question is
only why the gate never opens.

## Why the gate never opens

```rust
pub(crate) fn clean_reuse(&self) -> Option<(Vec<SegRef>, u64, u64)> {
    let data = self.inner.data.read();
    if data.dirty {
        return None;
    }
    data.base.as_ref().map(|b| (b.seg_refs.clone(), b.vector_count, b.tombstone_count))
}
```

Two conditions, and an ordinary collection fails both.

**`data.base` is the mmap base.** It is populated by exactly one caller,
`install_base`, and the load path reaches it only on the mmap tier
(`database.rs:148`):

```rust
if let Some(base) = loaded.base {
    handle.install_base(base)?;   // mmap tier (ADR-0004)
} else {
    // ... load points into memory
}
```

A collection small enough to fit the memory budget takes the `else` branch, so
`base` stays `None` and `clean_reuse` returns `None` regardless of anything else.

**`data.dirty` is set by loading.** The in-memory branch calls `install_points`
→ `apply_prepared` → `apply_upsert`, and `apply_upsert` ends with
(`collection.rs:2060`):

```rust
data.payload_indexes.insert(slot as u64, data.payloads[slot].as_ref());
data.dirty = true;
```

That is correct for a genuine write. It is wrong as a description of a load: at
that instant the in-memory state and the committed state are identical.

`dirty` is cleared in exactly one place — `install_base` — which is the mmap path
again. So a non-mmap collection is dirty from the moment it is loaded and stays
dirty forever, and every checkpoint reseals and rewrites all of its segments.

The reuse optimisation therefore only ever helps large mapped collections. Every
ordinary collection pays a full rewrite per checkpoint, and closing the database
checkpoints.

Reopening the database does not help, which is worth stating because it is the
obvious thing to try: verified, the growth continues, because a small collection
still takes the in-memory path.

## The refs already exist at commit time

The fix does not need new bookkeeping. `pager.checkpoint` already computes the
committed `SegRef`s — offset and length — as it writes, to build the TOC entry
(`pager.rs:220`):

```rust
let mut refs = Vec::with_capacity(c.segments.len());
for seg in &c.segments {
    let bytes = seg.encode(chosen)?;
    file.write_all(&bytes)?;
    refs.push(SegRef { seg_type: seg.seg_type.to_byte(), offset: cur, len: bytes.len() as u64 });
    cur += bytes.len() as u64;
}
```

They are simply never handed back to the collection. And on the load side the
same information is already in hand: the TOC entry's `live_segments` **are** the
committed refs for that collection.

So both moments where a collection is provably in sync with the file — right
after a commit, and right after a load — already know the refs. Neither records
them.

## Why `vacuum` does not save you

`vacuum` reclaims the space (241,108 → 47,176 bytes measured). But auto-vacuum
escalates on the **tombstone ratio** (STG-072), and an idle checkpoint creates no
tombstones, so it never fires. Only an explicit `vacuum` recovers the space, and
an operator has no reason to suspect it is needed — the database is idle and the
data has not changed.

## Consequence

Every application opens and closes the database once per run. A tool run daily
grows its file daily with nothing changing. A long-running process that
checkpoints periodically for durability — the pattern SPEC-003 encourages — grows
without bound while completely idle.
