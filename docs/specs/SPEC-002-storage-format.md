# SPEC-002 — Storage Format (`.veclite` v1)

| | |
|---|---|
| **Status** | **Frozen-normative** (format v1, phase2e / gate G2). The on-disk byte layout is stable: committed v1 golden files (`crates/veclite/tests/compat/golden/`) MUST keep opening on every future commit, and any change to the byte format requires a **new format version**, never an edit to v1. |
| **Phase / tasks** | Phase 2 · T2.1, T2.2, T2.5–T2.9 |
| **PRD requirements** | FR-03–06, FR-50, FR-53–56; NFR-05, NFR-11 |
| **Planning source** | [04-storage-format.md](../vectorizer-lite/04-storage-format.md) |

Requirement IDs `STG-xxx`. All multi-byte integers are **little-endian**. WAL and durability semantics are SPEC-003.

## 1. Design invariants

- **STG-001** One database = one `.veclite` file. All state — collection configs, vectors, HNSW graphs, payloads, payload indexes, sparse postings, embedding vocabularies — lives in this file. The only sidecar ever created is the transient `<name>-wal` (SPEC-003).
- **STG-002** Segments are **append-ordered and immutable once sealed**. Updates and deletes append new segments; space is reclaimed only by `vacuum`.
- **STG-003** The file is always recoverable: the header points at the last valid TOC; a crash mid-write can only lose the not-yet-committed tail, never damage committed state ("root pointer swap" discipline, §5).
- **STG-004** Vector segments are laid out for direct mmap access: fixed stride, encoding declared in the segment header, no decode required to address a vector (FR-53).

## 2. File header (offset 0, 4096 bytes, fixed)

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 4 | magic | ASCII `VECL` |
| 4 | 4 | `format_version: u32` | 1 |
| 8 | 4 | `min_reader_version: u32` | readers with support < this MUST fail with `UnsupportedFormatVersion` |
| 12 | 4 | header_crc32 | crc32 of bytes 0..12 and 16..256 (crc field zeroed) |
| 16 | 8 | `flags: u64` | bit 0 = `clean_close`; bits 1–63 reserved (MUST be 0 in v1) |
| 24 | 8 | `toc_offset: u64` | byte offset of the current TOC |
| 32 | 8 | `toc_len: u64` | |
| 40 | 4 | `toc_crc32: u32` | crc32 of the TOC bytes |
| 44 | 4 | reserved | 0 |
| 48 | 16 | `file_uuid: [u8; 16]` | random v4, assigned at creation, never changes |
| 64 | 8 | `created_epoch_s: u64` | |
| 72 | 8 | `modified_epoch_s: u64` | updated on header swap |
| 80 | 176 | reserved | MUST be zero in v1 |
| 256 | 3840 | reserved page tail | MUST be zero in v1; readers MUST ignore |

- **STG-010** A reader MUST validate magic, `header_crc32`, and `min_reader_version` before anything else. Invalid header → `Corrupt("header")`.
- **STG-011** Writers MUST rewrite the header with a single 4 KiB write followed by fsync (§5).

## 3. Segments

Every segment starts with a 32-byte header:

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | `seg_type: u8` (§3.1) |
| 1 | 1 | `compression: u8` — 0 none, 1 lz4, 2 zstd |
| 2 | 2 | `seg_flags: u16` (type-specific; 0 in v1 unless noted) |
| 4 | 4 | `coll_id: u32` (0xFFFF_FFFF for database-scope segments) |
| 8 | 8 | `body_len: u64` (compressed length as stored) |
| 16 | 8 | `uncompressed_len: u64` (= body_len when compression = 0) |
| 24 | 4 | `body_crc32: u32` (crc of the stored bytes) |
| 28 | 4 | reserved (0) |

- **STG-020** Bodies < 1024 bytes SHOULD NOT be compressed (server-parity threshold); compression is per segment, LZ4 default, zstd allowed.
- **STG-021** Readers MUST verify `body_crc32` before use; mismatch → `Corrupt("segment@<offset>")` naming the offset and type.

### 3.1 Segment types (v1)

| `seg_type` | Name | Body content (after decompression) |
|---|---|---|
| 1 | `CONFIG` | Collection config, MessagePack-encoded¹ — dimension, metric, hnsw params, quantization, compression, embedding_provider, declared payload indexes, creation time |
| 2 | `VECTORS` | Fixed-stride vector block, §3.2 |
| 3 | `TOMBSTONE` | Roaring bitmap (portable serialization) of deleted slot numbers |
| 4 | `PAYLOAD` | Payload block: sequence of `(slot u64, len u32, msgpack payload)`; auto-embed collections store original text under reserved key `_text` |
| 5 | `PIDX` | Payload index: kind byte (1 keyword / 2 int / 3 float) + key name + sorted postings (`value → roaring bitmap of slots`) |
| 6 | `SPARSE` | Sparse postings for the hybrid lane (SPEC-007 §4): `term_id u32 → postings (slot, weight f32)` |
| 7 | `HNSW` | Serialized graph: 1-byte graph-format version + `hnsw_rs` dump (layers, links, entry point) |
| 8 | `VOCAB` | Embedding provider state (opaque bytes from `Embedder::export_state`, SPEC-005) |
| 9 | `IDDIR` | id → slot directory, §3.3 |

¹ Resolves PRD OQ-5: **MessagePack everywhere** (config, payloads, WAL bodies, FFI codec) — one codec across the whole surface beats bincode's marginal size win.

- **STG-022** Unknown `seg_type` values: readers of format_version 1 MUST fail with `Corrupt`; future minor format revisions that add types MUST bump `min_reader_version` **unless** the segment is advisory (flagged by `seg_flags` bit 15 = "ignorable"), which v1 readers skip.

### 3.2 VECTORS segment body

Header (within body): `encoding u8` (0 f32 · 1 sq8 · 2 sq4 · 3 sq2 · 4 sq1 · 5 binary · 6 pq), `dimension u32`, `count u64`, `first_slot u64`, then encoding parameters:

- sq*: `scale f32, offset f32` (per segment);
- pq: codebook (feature `pq`; segments with encoding 6 on a build without `pq` → `UnsupportedProvider`-class error at open);
- then `count` records at fixed stride: f32 = `dimension × 4` bytes; sq8 = `dimension`; sq4/2/1 packed; binary = `dimension / 8` (dimension MUST be a multiple of 8 for binary).

- **STG-030** Slots are contiguous: segment covers `first_slot .. first_slot + count`. A vector's location = segment base + `(slot − first_slot) × stride`; mmap readers compute this without decoding (STG-004).
- **STG-031** VECTORS segments MUST NOT be compressed (compression = 0) — they are the mmap hot path. Payload/PIDX/SPARSE/VOCAB/CONFIG MAY be compressed.

### 3.3 IDDIR segment

- **STG-032** Maps string id → slot: hash-bucketed directory (xxhash64 of id → bucket → `(id bytes, slot u64)` entries). Collisions resolved within the bucket by full id comparison. Tombstoned slots remain in the directory until vacuum rewrites it.

## 4. Table of contents (TOC)

MessagePack document written as the last step of every checkpoint:

```
Toc {
  generation: u64,                    // monotonically increasing
  collections: [CollEntry],
  free_tail_offset: u64,              // next append position
}
CollEntry {
  coll_id: u32, name: str, aliases: [str],
  vector_count: u64, tombstone_count: u64,
  live_segments: [(seg_type u8, offset u64, len u64)],  // replay order
}
```

- **STG-040** The TOC lists **live** segments only; superseded segments (older HNSW generations, compacted-away VECTORS) simply stop being referenced and become dead space until vacuum.
- **STG-041** A collection's state is reconstructed by loading its live segments in the listed order; the order MUST be deterministic: CONFIG, IDDIR, VECTORS*, TOMBSTONE, PAYLOAD*, PIDX*, SPARSE*, VOCAB, HNSW.

## 5. Commit protocol (root-pointer swap)

- **STG-050** Checkpoint commit sequence (normative, in order): (1) append new segments; (2) fsync file; (3) append new TOC at the tail; (4) fsync file; (5) rewrite the 4 KiB header pointing at the new TOC; (6) fsync file. A crash between any steps leaves the previous header→TOC chain intact and valid.
- **STG-052** A checkpoint MUST NOT rewrite a collection whose state already matches the file: the new TOC references the committed segments in place (carry-forward), for every storage tier and not only for mmap'd collections. A checkpoint with nothing to persist — every collection carried forward and no WAL entry pending — MUST leave the file byte-length unchanged, so a process that checkpoints on a timer, or is merely opened and closed, does not grow its database while idle. Carried-forward references are offsets into **this** file: `vacuum` and `snapshot` write fresh files and MUST invalidate them. This constrains only what a checkpoint may rewrite; the commit sequence in STG-050 is unchanged, and the first commit on a new database always runs (there is nothing to carry forward yet).
- **STG-053** Database creation MUST materialize the initial generation (header + gen-0 TOC) in a sibling temp file and atomically rename it into place. A brand-new file has no previous header→TOC chain for STG-050's crash argument to fall back on — the TOC is appended before the header is written, so a crash inside the first commit would otherwise leave a zeroed header at the target path, permanently unopenable. With the rename, a crash mid-creation leaves nothing at the target path at all; a leftover creation temp is the writer's own artifact, removed on the next create.
- **STG-051** On open, if `toc_crc32` fails, the reader MUST fail with `Corrupt("toc")` — the header swap discipline makes this state unreachable except by external damage; there is no silent fallback in read-write mode. `read_only` open MAY be extended post-v1 to scan for a previous TOC generation; v1 requirement is only that a **damaged tail beyond the committed TOC** never affects opening (STG-003, test §9.3).

## 6. Concurrency, locking, read-only

- **STG-060** Read-write open takes an exclusive advisory lock (`fd-lock`) on the `.veclite` file; `read_only` open takes a shared lock. Lock conflict → `Locked` immediately (no blocking wait).
- **STG-061** In-process readers proceed during checkpoint thanks to segment immutability; the TOC pointer swap happens under a brief per-database write lock (the only cross-collection stop-the-world point, target < 1 ms).
- **STG-062** `read_only` mode MUST: skip WAL replay if a WAL exists but is empty/clean; **refuse to open** with `Locked` if a live writer holds the exclusive lock is fine to read (shared lock acquisition succeeds only against other readers or a `Normal`-durability quiesced file); refuse all mutating calls with `ReadOnly`.
- **STG-063** HNSW load (v1, [ADR-0004](../../.rulebook/decisions/004-single-file-mmap-vectors-with-exact-brute-force-larger-than-ram-tier.md)): v1 does **not** persist the HNSW graph — `hnsw_rs`'s on-disk format is version-unstable, and its dump/reload works only over a directory of its own files, which cannot live inside the single `.veclite`. Open therefore **always rebuilds** the graph from the (mmap'd) VECTORS via parallel insert; the `HNSW` segment stays reserved (byte 7) but carries no graph in v1. The `OpenOptions` warning callback (FR-54) is retained in the API for a future persisted-graph path and is not fired in v1. Non-VECTORS segment corruption remains fatal (`Corrupt`). Reframes, without changing any on-disk bytes, the earlier "load graph, rebuild on crc miss" contract.
- **STG-064** Larger-than-RAM tier ([ADR-0004](../../.rulebook/decisions/004-single-file-mmap-vectors-with-exact-brute-force-larger-than-ram-tier.md)): when a collection's mmap'd VECTORS exceed a memory budget, open skips the in-RAM HNSW build and serves **exact** k-NN by SIMD brute-force scan over the mmap'd fixed-stride records (recall is exact, cost O(n·dim) per query). Below the budget, the graph is rebuilt in RAM and ANN search is served as before. Results below the budget match the pre-mmap path; above it they are exact.

## 7. Snapshot and vacuum

- **STG-070** `snapshot(path)`: run a checkpoint, then copy header + live segments + fresh TOC into a **new compacted file** at `path` (dead space and tombstoned slots dropped, IDDIR rewritten). The snapshot is a valid standalone `.veclite` file with a **new** `file_uuid`. Writers are not blocked beyond the checkpoint's TOC swap window.
- **STG-071** `vacuum()`: (1) checkpoint; (2) rewrite live data of collections exceeding the tombstone threshold into fresh segments appended to the same file; (3) write new TOC + header swap; (4) truncate the file tail. On Windows, the pager MUST unmap→truncate→remap (mapped regions cannot be truncated).
  - **v1 implementation note** (phase2d, ADR-0003): while there is no memory-mapped read path (mmap is deferred to phase2f), v1 `vacuum()` shrinks via a compacted **temp file + atomic close→rename→reopen** swap (preserving `file_uuid`) rather than in-place append-then-truncate. This is crash-safe (a crash leaves either the original+WAL or the compacted file, both valid) and Windows-safe (the handle is closed before the rename). In-memory readers are served from RAM, so none are invalidated. The in-place append-then-truncate with unmap→truncate→remap becomes relevant only once an active mmap exists (phase2f).
- **STG-072** Auto-vacuum: when a collection's tombstones exceed 25 % of slots (tunable via `OpenOptions`), the next checkpoint escalates to a vacuum of that collection.

## 8. Limits (format v1)

| Limit | Value | Enforcement |
|---|---|---|
| Max file size | 2^63 bytes | u64 offsets |
| Max collections | 2^32 | coll_id width |
| Max vectors per collection | 2^40 | slot directory width |
| Max dimension | 65 536 | `InvalidArgument` at create |
| Max payload | 16 MiB compressed | `InvalidArgument` at upsert |
| Max id length | 512 bytes UTF-8 | `InvalidArgument` at upsert |

## 9. Acceptance criteria (gate G2)

1. **Round-trip**: every segment type encode→decode property-tested; TOC generation monotonicity asserted.
2. **Crash suite** (T2.10, SPEC-015 §4): 10 000 iterations of kill-9 + fault-injection torn writes — reopen always succeeds, all acked-`Full` commits present, zero main-file corruption.
3. **Bit-flip drills**: random single-bit corruption in each segment type → open fails with `Corrupt` naming the segment; never UB, never a wrong answer. Damaged tail beyond committed TOC → `read_only` and rw open both succeed.
4. **mmap**: dataset 4× RAM opens and serves searches; warm open of the 1 M-vector reference file < 100 ms (NFR-02).

**Freeze status (phase2e).** Criteria 1–3 are met and enforced in CI: the crash
suite (`crates/veclite/tests/crash_safety.rs` + `cargo xtask crash`) runs 10 000
in-process iterations plus a real subprocess kill-9 harness nightly on
Linux/macOS/Windows, and the v1 golden files are guarded on every run
(`crates/veclite/tests/golden.rs`). These fix the **byte format**, which is now
frozen. Criterion 4 (the mmap access path and its warm-open budget) does not
change any on-disk bytes and was delivered in `phase2f_mmap-hnsw-persistence`
(ADR-0004): STG-004 mmap addressing with the STG-064 tier split, reading the
same frozen v1 layout; the at-scale 4×RAM run lands with the phase-6 soak
(DAG T6.2).
5. **Windows vacuum**: shrink-in-place under mmap passes on Windows CI.
6. **Freeze artifact**: this document marked frozen-normative; golden files for v1 committed to `tests/compat/golden/` and read by every subsequent CI run (NFR-11).
