# Storage format (`.veclite` v1)

A VecLite database is **one file**. The format is **frozen at v1** — every
future 1.x release reads files written today (see [format
stability](versioning.md)). This page is the operator-level summary; the
byte-exact normative spec is [SPEC-002](../../specs/SPEC-002-storage-format.md),
guarded by committed golden files.

## Layout

```
app.veclite
├── 4 KiB header          root pointer (magic, format version, TOC offset, CRC)
├── immutable segments    append-only; each CRC-framed, never rewritten in place
│   ├── CONFIG            per-collection config (dimension, metric, HNSW, …)
│   ├── VECTORS           fixed-stride vector block (mmap-addressable)
│   ├── IDDIR             id → slot directory (xxhash-bucketed)
│   ├── PAYLOAD           per-point payloads (MessagePack)
│   ├── PIDX              declared payload indexes (roaring bitmaps)
│   ├── SPARSE            sparse lane for hybrid search
│   ├── TOMBSTONE         deleted slots
│   └── VOCAB             auto-embed vocabulary state
└── MessagePack TOC       lists the live segments of every collection

app.veclite-wal            write-ahead log sidecar (crash recovery)
```

## Commit protocol

A checkpoint appends new immutable segments, fsyncs, appends a new TOC, fsyncs,
then rewrites the 4 KiB header to point at it and fsyncs again. Because nothing
committed is ever overwritten — only the header is rewritten in place, atomically
— a crash between any two steps leaves the previous header→TOC chain intact. The
write-ahead log ([SPEC-003](../../specs/SPEC-003-wal-durability.md)) captures
uncommitted writes; on a non-clean close it replays on top of the last
checkpoint. **kill-9 never corrupts the file** — proven by the crash suite
(`cargo xtask crash`, randomized workloads against an oracle plus a real
subprocess kill harness).

## Integrity

Every segment carries a CRC over its stored bytes; the header and TOC carry
their own CRCs. The `veclite verify` command runs a full read-only integrity
pass — header, TOC, every segment, the WAL scan, and the id-directory
consistency check — naming any damaged segment by offset and type. See the
[CLI guide](../guides/cli.md).

## Portability

The same bytes are produced and consumed on every platform and by the WASM
in-memory image codec (`serialize` / `deserialize`): a `.veclite` image written
in a browser opens byte-for-byte in native VecLite. The format is independent of
CPU endianness and word size within the documented limits.

## Relationship to `.vecdb`

VecLite's `.veclite` is **not** the Vectorizer server's `.vecdb` — they are
different files for different deployment models. The
[graduation](../guides/graduation.md) and [reverse](../guides/reverse-migration.md)
guides bridge them losslessly.
