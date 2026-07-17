//! The full-image codec: a `.veclite` v1 file assembled in / parsed from a
//! `Vec<u8>` instead of a `std::fs::File` (SPEC-002 §2/§4/§5). This is the exact
//! byte layout the [`crate::storage::pager::Pager`] writes — `[4 KiB header]
//! [segments…][TOC]`, offsets assigned monotonically from 4096 — but with no
//! filesystem, mmap, or locks, so it compiles and runs on wasm32 (CORE-004).
//! It is the interchange contract (WASM-010): an image written here opens with
//! native VecLite, and a native file's committed bytes parse here.
//!
//! Every primitive below the pager (`Segment::encode`/`read`, `Header`, `Toc`,
//! `compression` with LZ4/none) is already pure; this module just replays the
//! pager's offset-assignment loop over an in-memory buffer. The native pager and
//! this codec share those primitives, so their output is byte-compatible by
//! construction (a native test asserts it, STG-050).

use crate::error::{Result, VecLiteError};
use crate::storage::compression::Codec;
use crate::storage::header::{FLAG_CLEAN_CLOSE, HEADER_SIZE, Header};
use crate::storage::segment::{Segment, codec_for};
use crate::storage::toc::{CollEntry, SegRef, Toc};

/// One collection's contribution to a checkpoint/image: its metadata plus the
/// new segments to append. The writer assigns offsets and builds the TOC entry.
/// Shared by the native pager and the portable image writer.
pub(crate) struct CheckpointColl {
    pub(crate) coll_id: u32,
    pub(crate) name: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) vector_count: u64,
    pub(crate) tombstone_count: u64,
    pub(crate) segments: Vec<Segment>,
    /// Carry-forward (ADR-0004): when set, the collection is unchanged since its
    /// segments were committed — the new TOC references them in place and
    /// nothing is rewritten. Valid only within the same file (segments are
    /// immutable, STG-002); never set for snapshot/vacuum/image targets, whose
    /// fresh buffers invalidate every offset. `segments` must be empty when set.
    pub(crate) reused: Option<Vec<SegRef>>,
}

/// A collection recovered from an image: its TOC entry plus its decoded (CRC-
/// checked, decompressed) segments, ready for `seal::load`.
pub(crate) struct ImageColl {
    pub(crate) entry: CollEntry,
    pub(crate) segments: Vec<Segment>,
}

fn as_usize(v: u64, ctx: &str) -> Result<usize> {
    usize::try_from(v).map_err(|_| VecLiteError::Corrupt(format!("{ctx}: offset exceeds usize")))
}

/// Assemble a complete single-generation `.veclite` v1 image (SPEC-002 §5): the
/// header page, the appended segments, and the trailing TOC the header points
/// at. Mirrors [`crate::storage::pager::Pager::checkpoint`] byte-for-byte, with
/// the file replaced by a `Vec<u8>` and the fsyncs elided (there is no disk).
pub(crate) fn write_image(
    file_uuid: [u8; 16],
    created_epoch_s: u64,
    modified_epoch_s: u64,
    generation: u64,
    colls: Vec<CheckpointColl>,
    codec: Codec,
) -> Result<Vec<u8>> {
    // The header occupies the first page; segments start at HEADER_SIZE and the
    // buffer length tracks the next-append offset exactly (as the pager's tail).
    let mut buf = vec![0u8; HEADER_SIZE];
    let mut entries = Vec::with_capacity(colls.len());
    for c in colls {
        // Carry-forward references prior committed offsets in an existing file;
        // a fresh image has none, so the image path always seals every segment.
        if c.reused.is_some() {
            return Err(VecLiteError::Corrupt(format!(
                "image: collection {:?} was carried forward (reused refs) but an \
                 image seals every segment fresh",
                c.name
            )));
        }
        let mut refs = Vec::with_capacity(c.segments.len());
        for seg in &c.segments {
            let chosen = codec_for(seg.seg_type, codec, seg.body.len());
            let bytes = seg.encode(chosen)?;
            let offset = buf.len() as u64;
            buf.extend_from_slice(&bytes);
            refs.push(SegRef {
                seg_type: seg.seg_type.to_byte(),
                offset,
                len: bytes.len() as u64,
            });
        }
        let mut entry = CollEntry {
            coll_id: c.coll_id,
            name: c.name,
            aliases: c.aliases,
            vector_count: c.vector_count,
            tombstone_count: c.tombstone_count,
            live_segments: refs,
        };
        entry.sort_replay_order();
        entries.push(entry);
    }

    let toc_start = buf.len() as u64;
    let toc = Toc {
        generation,
        collections: entries,
        free_tail_offset: toc_start,
    };
    let tbytes = toc.encode()?;
    buf.extend_from_slice(&tbytes);

    let mut header = Header::new(file_uuid, created_epoch_s);
    header.flags = FLAG_CLEAN_CLOSE;
    header.toc_offset = toc_start;
    header.toc_len = tbytes.len() as u64;
    header.toc_crc32 = crc32fast::hash(&tbytes);
    header.modified_epoch_s = modified_epoch_s;
    buf[0..HEADER_SIZE].copy_from_slice(&header.encode());
    Ok(buf)
}

/// Parse a committed `.veclite` v1 image (SPEC-002 §5, STG-051): decode and
/// validate the header, CRC-check and decode the TOC it points at, then read
/// (CRC-check + decompress) every live segment of every collection. Mirrors the
/// pager's `read_committed` + per-segment `read_segment` over a byte slice.
pub(crate) fn read_image(bytes: &[u8]) -> Result<Vec<ImageColl>> {
    if bytes.len() < HEADER_SIZE {
        return Err(VecLiteError::Corrupt(
            "image: shorter than 4 KiB".to_owned(),
        ));
    }
    let header = Header::decode(&bytes[..HEADER_SIZE])?;

    let toc_off = as_usize(header.toc_offset, "toc")?;
    let toc_len = as_usize(header.toc_len, "toc")?;
    let tbuf = bytes
        .get(toc_off..toc_off.saturating_add(toc_len))
        .ok_or_else(|| VecLiteError::Corrupt("image: TOC out of range".to_owned()))?;
    if crc32fast::hash(tbuf) != header.toc_crc32 {
        return Err(VecLiteError::Corrupt("image: TOC crc mismatch".to_owned()));
    }
    let toc = Toc::decode(tbuf)?;

    let mut out = Vec::with_capacity(toc.collections.len());
    for entry in toc.collections {
        let mut segments = Vec::with_capacity(entry.live_segments.len());
        for seg in &entry.live_segments {
            let off = as_usize(seg.offset, "segment")?;
            let len = as_usize(seg.len, "segment")?;
            let sbuf = bytes.get(off..off.saturating_add(len)).ok_or_else(|| {
                VecLiteError::Corrupt(format!("image: segment@{} out of range", seg.offset))
            })?;
            segments.push(Segment::read(sbuf, 0, seg.offset)?.0);
        }
        out.push(ImageColl { entry, segments });
    }
    Ok(out)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::storage::pager::Pager;
    use crate::storage::segment::SegmentType;

    fn seg(t: SegmentType, coll_id: u32, body: Vec<u8>) -> Segment {
        Segment {
            seg_type: t,
            seg_flags: 0,
            coll_id,
            body,
        }
    }

    /// A zero-uuid, zero-epoch image (exactly what the wasm `serialize` path
    /// emits, WASM-010) must open with the native file pager — the interchange
    /// direction the wasm binding relies on.
    #[test]
    fn zero_uuid_image_opens_with_native_pager() {
        let colls = vec![CheckpointColl {
            coll_id: 0,
            name: "docs".to_owned(),
            aliases: vec![],
            vector_count: 1,
            tombstone_count: 0,
            segments: vec![
                seg(SegmentType::Config, 0, b"cfg-body-placeholder".to_vec()),
                seg(SegmentType::Vectors, 0, vec![1, 2, 3, 4]),
            ],
            reused: None,
        }];
        let bytes = write_image([0u8; 16], 0, 0, 1, colls, Codec::Lz4)
            .unwrap_or_else(|e| panic!("write_image: {e}"));

        let path = std::env::temp_dir().join(format!(
            "veclite-image-zerouuid-{}.veclite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("write file: {e}"));
        {
            let (_, toc) = Pager::open(&path, true).unwrap_or_else(|e| panic!("pager open: {e}"));
            assert_eq!(toc.generation, 1);
            assert_eq!(toc.collections.len(), 1);
            assert_eq!(toc.collections[0].name, "docs");
        }
        let _ = std::fs::remove_file(&path);
    }

    /// `write_image` then `read_image` reproduces the segments byte-for-byte
    /// (CONFIG/VECTORS bodies survive the compression policy), the pure-memory
    /// direction the wasm `deserialize` relies on.
    #[test]
    fn write_then_read_round_trips_segments() {
        let colls = vec![CheckpointColl {
            coll_id: 3,
            name: "c".to_owned(),
            aliases: vec!["a".to_owned()],
            vector_count: 1,
            tombstone_count: 0,
            segments: vec![
                seg(SegmentType::Config, 3, vec![9; 2048]),
                seg(SegmentType::Vectors, 3, vec![7, 7, 7, 7]),
            ],
            reused: None,
        }];
        let bytes = write_image([1u8; 16], 5, 5, 2, colls, Codec::Lz4)
            .unwrap_or_else(|e| panic!("write_image: {e}"));
        let back = read_image(&bytes).unwrap_or_else(|e| panic!("read_image: {e}"));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].entry.name, "c");
        assert_eq!(back[0].entry.aliases, vec!["a".to_owned()]);
        // Segments come back in replay order (CONFIG before VECTORS) with bodies
        // intact after the compress/decompress round-trip.
        let cfg = &back[0].segments[0];
        assert_eq!(cfg.seg_type, SegmentType::Config);
        assert_eq!(cfg.body, vec![9; 2048]);
        let vecs = &back[0].segments[1];
        assert_eq!(vecs.seg_type, SegmentType::Vectors);
        assert_eq!(vecs.body, vec![7, 7, 7, 7]);
    }

    #[test]
    fn corrupt_image_is_reported_not_panic() {
        assert!(matches!(
            read_image(&[0u8; 10]),
            Err(VecLiteError::Corrupt(_))
        ));
        // A valid header whose TOC offset points past the buffer.
        let ok = write_image([0u8; 16], 0, 0, 1, Vec::new(), Codec::Lz4)
            .unwrap_or_else(|e| panic!("write_image: {e}"));
        let mut torn = ok.clone();
        torn.truncate(ok.len() - 1);
        assert!(matches!(read_image(&torn), Err(VecLiteError::Corrupt(_))));
    }
}
