# phase2a .veclite format-v1: commit protocol, roaring treemap slots, free_tail non-circularity
**Source**: manual
**Date**: 2026-07-14
**Related Task**: phase2a_pager-segments
**Tags**: storage, rust, crash-safety, roaring, msgpack, phase2a
SPEC-002 storage layer (native-only, gated off wasm32 — CORE-004; zstd links C):

1. Deps (native-only under [target.'cfg(not(target_arch="wasm32"))'.dependencies]): rmp-serde (MessagePack, OQ-5), lz4_flex 0.13 + zstd 0.13 (vendored wrappers: lz4 = compress_prepend_size, zstd = stream::encode_all level 3 — matches vectorizer-core for .vecdb byte-compat), crc32fast, roaring 0.10, xxhash-rust (xxh64 for IDDIR), uuid v4 (file_uuid). None are network crates; the deny-list check `cargo tree -e normal` excludes dev+wasm anyway. Verified wasm32 build stays green (storage absent).

2. Module layout src/storage/: header.rs (4KiB, crc over [0,12)+[16,256)), compression.rs (Codec enum), segment.rs (32B header + framing + per-segment crc; codec_for policy: None for VECTORS/STG-031 or <1024 bytes/STG-020), toc.rs (rmp-serde, SegmentType::replay_rank for STG-041 ordering), pager.rs (commit protocol), vectors.rs (fixed-stride, slot addressing = (slot-first_slot)*stride), iddir.rs (hash-bucketed), body.rs (CONFIG/PAYLOAD/PIDX/SPARSE/VOCAB/HNSW + tombstone).

3. Commit protocol (STG-050) in Pager::checkpoint: seek to tail, write all segments recording SegRef offsets, sync_all (fsync #1), write TOC at cur, sync_all (#2), rewrite 4KiB header pointing at new TOC + toc_crc32, seek(0) write header, sync_all (#3). Append-only — never overwrites committed data; only the header is rewritten in place. On open, next-append tail = header.toc_offset + header.toc_len (authoritative). free_tail_offset in the TOC is set to toc_start (= end of segments) to AVOID the circularity of storing after-TOC-offset inside the TOC (msgpack uint width varies with value). The header is the source of truth for the tail.

4. Slot bitmaps use roaring::RoaringTreemap (u64), not RoaringBitmap (u32), to match the u64 slot space (STG limit 2^40 vectors/collection). serialize_into(&mut Vec)/deserialize_from(Cursor).

5. `gen` is a RESERVED KEYWORD in edition 2024 — can't be a variable name. Cast lints (cast_possible_truncation) are pedantic and NOT enabled by the workspace (only unwrap_used/expect_used denied), so `x as usize` is fine; length checks (filter e<=buf.len()) guard bad reads. Every decode returns VecLiteError::Corrupt(locator) never panics — proptest fuzz (arbitrary bytes into every decoder) confirms. Crash test: append garbage to the tail after a committed checkpoint → reopen still returns the previous generation (STG-003). Phase2a is the codec+commit protocol only; wiring into VecLite::open (WAL, mmap, locking) is phase2b/2c.