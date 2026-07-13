# 04 — Storage Format (`.veclite`)

## Requirements

1. **Single file.** Everything — collection configs, vectors, HNSW graphs, payloads, payload indexes, embedding vocabularies — in one file. (The server's `.vecdb` uses a sidecar `.vecidx` JSON index plus a `snapshots/` directory; VecLite folds all of it in.)
2. **Crash-safe.** `kill -9` at any moment leaves the file recoverable: either the write is fully applied after WAL replay or fully absent. Never a corrupt main file.
3. **mmap-friendly reads.** Vector segments are laid out for direct `memmap2` access so datasets larger than RAM page in on demand (inherits the design of `storage/mmap.rs`).
4. **Versioned + checksummed.** Format version in the header; per-segment crc32 (`crc32fast`). Newer readers accept all older versions; older readers fail with `UnsupportedFormatVersion`.
5. **Append-mostly.** Updates and deletes append; a compactor (`vacuum`) reclaims space. Enables lock-light concurrent readers via segment immutability.

## File layout

```
┌────────────────────────────────────────────────┐
│ Header (4 KiB, fixed)                          │
│   magic  = "VECL"           (4 B)              │
│   format_version u32        (starts at 1)      │
│   min_reader_version u32                       │
│   flags u64                 (bit0: clean_close)│
│   toc_offset u64, toc_len u64, toc_crc32 u32   │
│   file_uuid [16]B, created/modified u64 epoch  │
│   reserved → zero                              │
├────────────────────────────────────────────────┤
│ Segments (append-ordered, immutable once       │
│ sealed; each: type u8, coll_id u32, len u64,   │
│ crc32, compression u8 [none|lz4|zstd], body)   │
│                                                │
│   CONFIG    collection config (bincode 2)      │
│   VECTORS   fixed-stride vector block —        │
│             f32 or SQ-8/4/2/1 or binary codes  │
│             (+ PQ codebook when enabled)       │
│   TOMBSTONE deleted id-set (roaring bitmap)    │
│   PAYLOAD   payload blocks (msgpack, LZ4)      │
│   PIDX      payload index (keyword/int/float)  │
│   SPARSE    sparse postings (BM25 / hybrid)    │
│   HNSW      serialized graph (layers, links)   │
│   VOCAB     embedding vocabulary (BM25/TF-IDF) │
├────────────────────────────────────────────────┤
│ TOC (table of contents, at toc_offset)         │
│   per collection: id, name, aliases,           │
│   live segment list (offset, len, type),       │
│   vector count, id→slot directory ref          │
└────────────────────────────────────────────────┘
```

Notes:

- **TOC is written last, then the header is atomically updated** (write new TOC at end → fsync → rewrite 4 KiB header pointing at it → fsync). A torn TOC write is harmless: the header still points at the previous valid TOC. This is the same "root pointer swap" that makes LMDB/SQLite-WAL robust.
- **VECTORS stride**: raw f32 (`dimension × 4` bytes), SQ-8 (`dimension` bytes; scale/offset per segment header), packed 4/2/1-bit, or binary (`dimension / 8`). Stride and encoding are declared in the segment header so mmap readers compute offsets without decoding — mirrors `ScalarQuantization`'s packing in `vectorizer-core`.
- **HNSW graph** is persisted (not rebuilt on open): open cost is mmap + TOC parse. `hnsw_rs 0.3` graphs serialize via the same dump/load path used by the server's snapshot code; if the graph segment is missing/corrupt, VecLite falls back to a rebuild from vectors (slow-open with a warning callback in `OpenOptions`).
- **id → slot directory**: string ids hash into a compact directory segment; slots index the fixed-stride vector block.

## Write path & WAL

Sidecar `app.veclite-wal` (same directory, SQLite naming convention):

```
WAL entry: seq u64 | coll_id u32 | op u8 | len u32 | crc32 | msgpack body
ops: UPSERT_BATCH, DELETE_BATCH, CREATE_COLL, DROP_COLL, RENAME, ALIAS, VOCAB_UPDATE
```

- Every mutating API call appends one WAL entry (batch = one entry) + fsync (`OpenOptions::durability(Durability::Full | Normal | Off)`; default `Normal` = fsync on checkpoint + close, matching common embedded-DB practice; `Full` = fsync per commit).
- In-memory state (HNSW, payload maps) updates immediately after the WAL append — readers see writes at once.
- **Checkpoint** = seal in-memory deltas into new segments, write new TOC, swap header, truncate WAL. Triggered by: WAL size threshold (default 64 MiB, matches server checkpoint threshold pattern in `persistence/wal.rs`), explicit `db.checkpoint()`, or clean close.
- **Recovery on open**: header `clean_close` unset → replay WAL entries with valid crc32 in sequence order; a torn tail entry is discarded (partial batch never applied — the whole entry is the atomic unit).

## Concurrency & the file lock

- One OS advisory lock (`fd-lock`) taken exclusive on read-write open, shared on `read_only` open. Second writer process → `Error::Locked` immediately (no blocking waits by default).
- In-process: segment immutability lets readers proceed during checkpoint; a checkpoint swaps the TOC pointer under a brief write lock.
- `snapshot(path)`: checkpoint, then copy live segments + fresh TOC into a new compacted file. Consistent because sealed segments never mutate.

## Compaction (`vacuum`)

Deleted vectors accumulate as tombstones; HNSW nodes are soft-deleted (excluded from results, purged on reindex). `vacuum`:

1. Checkpoint the WAL.
2. Rewrite live data into fresh segments (dropping tombstoned slots, rewriting the id directory), same file, appended.
3. Swap the TOC, then truncate the tail — file shrinks in place. (Windows note: truncation of an mmap'd region requires remap; the pager handles unmap→truncate→remap.)

Auto-vacuum threshold: when tombstones exceed 25 % of a collection's slots (tunable), the next checkpoint escalates to a per-collection vacuum.

## Limits (format v1)

| Limit | Value |
|---|---|
| Max file size | 2^63 bytes (offsets are u64) |
| Max collections | 2^32 |
| Max vectors per collection | 2^40 (slot directory width) |
| Max dimension | 65 536 |
| Max payload size | 16 MiB per vector (compressed) |
| String id length | 512 bytes UTF-8 |

## Relationship to the server's `.vecdb`

`.veclite` is **not** byte-compatible with `.vecdb` — it's a redesign meeting the single-file requirement. Interop is at the logical level via the `vecdb-interop` feature: export writes a `.vecdb` archive + `.vecidx` the server's `StorageReader`/`StorageMigrator` accepts; import reads both `.vecdb` layouts (`detect_format`: Legacy `*_vector_store.bin` and Compact). Quantized segments translate losslessly because both sides use `vectorizer-core`'s encodings. See [07-vectorizer-compatibility.md](07-vectorizer-compatibility.md).

## Crash-safety test plan (normative for 1.0)

1. **Torn-write fuzzing**: run a write workload under a fault-injection VFS shim that kills the process after every N bytes written; reopen and assert invariants (all acked-`Full` commits present; file never unreadable).
2. **Property tests** on WAL replay: arbitrary interleavings of upsert/delete/create/drop replayed = in-memory model state.
3. **crc corruption drills**: flip random bits in segments; open must fail with `Corrupt` naming the segment, never UB — and `read_only` open of the previous TOC generation must still succeed when only the tail is damaged.
