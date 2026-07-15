//! Memory-mapped read path over the `.veclite` file (SPEC-002 STG-004,
//! ADR-0004). For collections whose VECTORS exceed the in-RAM budget, the file
//! is mapped once and vectors are addressed straight out of the mapping by
//! stride — no decode, no per-vector copy, so a dataset larger than RAM pages
//! in on demand instead of being fully resident.
//!
//! This is a read-only view of *committed* segments. It never writes, and it
//! holds a second (read-only) handle on the file independent of the pager's
//! read-write handle. On Windows an active mapping blocks rename/truncate, so
//! `vacuum` drops the map before swapping the file and remaps afterwards
//! (STG-071); that coordination lives in the persistence layer.

use std::path::Path;
use std::sync::Arc;

use memmap2::Mmap;

use crate::error::{Result, VecLiteError};
use crate::storage::compression::Codec;
use crate::storage::segment::{SEGMENT_HEADER_SIZE, SegmentType};
use crate::storage::toc::SegRef;
use crate::storage::vectors::{Encoding, VectorsView};

/// A read-only memory map of an entire `.veclite` file. Segment bodies are
/// borrowed directly from the mapping; the borrow is tied to `&self`, so the
/// map cannot be dropped while a view into it is alive.
pub(crate) struct FileMap {
    mmap: Mmap,
}

impl FileMap {
    /// Map `path` read-only in its entirety. Fails with `Io` if the file cannot
    /// be opened or mapped.
    pub(crate) fn map(path: &Path) -> Result<FileMap> {
        let file = std::fs::OpenOptions::new().read(true).open(path)?;
        // SAFETY: we open a read-only mapping of a file we do not mutate through
        // this handle. The only in-process writer is the pager, which appends
        // (never overwrites committed bytes, STG-002/003) and, on vacuum,
        // requires this map to be dropped first (STG-071). Concurrent external
        // truncation is prevented by the pager's advisory lock (STG-060). The
        // mapping therefore observes stable committed bytes for its lifetime.
        let mmap = unsafe { Mmap::map(&file) }?;
        Ok(FileMap { mmap })
    }

    /// The full mapped byte length (i.e. the file size at map time).
    pub(crate) fn len(&self) -> usize {
        self.mmap.len()
    }

    /// The decompressed body of a segment referenced by `seg`, borrowed from the
    /// mapping. Only uncompressed segments are addressable this way; VECTORS are
    /// never compressed (STG-031), which is the intended caller. Verifies the
    /// segment's stored-body CRC before returning (STG-021) — corruption is
    /// `Corrupt`, never UB or a wrong answer.
    fn segment_body(&self, seg: SegRef) -> Result<&[u8]> {
        let buf = &self.mmap[..];
        let at = usize::try_from(seg.offset)
            .map_err(|_| VecLiteError::Corrupt("mmap: segment offset exceeds usize".to_owned()))?;
        let loc = || format!("segment@{}", seg.offset);
        let header_end = at
            .checked_add(SEGMENT_HEADER_SIZE)
            .filter(|&e| e <= buf.len())
            .ok_or_else(|| VecLiteError::Corrupt(format!("{}: truncated header", loc())))?;
        let codec = Codec::from_byte(buf[at + 1])?;
        if codec != Codec::None {
            return Err(VecLiteError::Corrupt(format!(
                "{}: mmap read path requires an uncompressed body",
                loc()
            )));
        }
        let stored_len = usize::try_from(u64::from_le_bytes([
            buf[at + 8],
            buf[at + 9],
            buf[at + 10],
            buf[at + 11],
            buf[at + 12],
            buf[at + 13],
            buf[at + 14],
            buf[at + 15],
        ]))
        .map_err(|_| VecLiteError::Corrupt(format!("{}: body length exceeds usize", loc())))?;
        let body_crc32 =
            u32::from_le_bytes([buf[at + 24], buf[at + 25], buf[at + 26], buf[at + 27]]);
        let body_end = header_end
            .checked_add(stored_len)
            .filter(|&e| e <= buf.len())
            .ok_or_else(|| VecLiteError::Corrupt(format!("{}: body past end of file", loc())))?;
        let body = &buf[header_end..body_end];
        if crc32fast::hash(body) != body_crc32 {
            return Err(VecLiteError::Corrupt(format!(
                "{}: body crc mismatch",
                loc()
            )));
        }
        Ok(body)
    }

    /// A borrowing [`VectorsView`] over the VECTORS segment referenced by `seg`.
    /// Rejects a `seg` whose type tag is not VECTORS (`Corrupt`).
    pub(crate) fn vectors_view(&self, seg: SegRef) -> Result<VectorsView<'_>> {
        if SegmentType::from_byte(seg.seg_type)? != SegmentType::Vectors {
            return Err(VecLiteError::Corrupt(format!(
                "segment@{}: expected VECTORS for mmap view",
                seg.offset
            )));
        }
        VectorsView::parse(self.segment_body(seg)?)
    }

    /// Build a long-lived [`VectorsRegion`] over the VECTORS segment referenced
    /// by `seg`. The body CRC is verified **once** here (STG-021); the region
    /// then addresses records by stored offsets with no per-access re-hash.
    /// Only the `F32` encoding is served by the v1 mmap tier (the encoding seal
    /// writes); others are `Corrupt`.
    pub(crate) fn vectors_region(self: &Arc<Self>, seg: SegRef) -> Result<VectorsRegion> {
        let view = self.vectors_view(seg)?;
        if view.encoding != Encoding::F32 {
            return Err(VecLiteError::Corrupt(format!(
                "segment@{}: mmap tier requires the F32 encoding",
                seg.offset
            )));
        }
        // Translate the view's record block into absolute map offsets: the
        // borrow starts somewhere inside `self.mmap`; recover its position.
        let base = self.mmap.as_ptr() as usize;
        let records = view
            .record(view.first_slot)
            .map(|r| r.as_ptr() as usize - base)
            .unwrap_or(self.mmap.len()); // empty segment: no records to address
        Ok(VectorsRegion {
            map: Arc::clone(self),
            records_start: records,
            stride: view.stride(),
            dimension: view.dimension,
            first_slot: view.first_slot,
            count: view.count,
        })
    }
}

/// A validated, long-lived window over one mmap'd VECTORS segment (STG-004,
/// ADR-0004). Holds the [`FileMap`] alive via `Arc`; addressing is offset
/// arithmetic into the mapping — no decode, no CRC re-hash, no copy until the
/// caller decodes a record's `f32`s.
pub(crate) struct VectorsRegion {
    map: Arc<FileMap>,
    /// Absolute byte offset of the first record within the mapping.
    records_start: usize,
    stride: usize,
    dimension: u32,
    first_slot: u64,
    count: u64,
}

impl VectorsRegion {
    pub(crate) fn first_slot(&self) -> u64 {
        self.first_slot
    }

    pub(crate) fn count(&self) -> u64 {
        self.count
    }

    pub(crate) fn dimension(&self) -> u32 {
        self.dimension
    }

    /// Total record bytes this region serves (the resident-memory cost it
    /// avoids).
    pub(crate) fn byte_len(&self) -> u64 {
        self.count * self.stride as u64
    }

    /// Raw record bytes for `slot`, or `None` outside
    /// `first_slot .. first_slot + count`.
    pub(crate) fn record(&self, slot: u64) -> Option<&[u8]> {
        let index = slot.checked_sub(self.first_slot)?;
        if index >= self.count {
            return None;
        }
        let start = self
            .records_start
            .checked_add(usize::try_from(index).ok()?.checked_mul(self.stride)?)?;
        let end = start.checked_add(self.stride)?;
        self.map.mmap.get(start..end)
    }

    /// Decode `slot`'s `f32` components into `out` (cleared first). Returns
    /// `false` for an out-of-range slot. The little-endian decode is the only
    /// copy on this path, into a caller-reused scratch buffer.
    pub(crate) fn read_f32_into(&self, slot: u64, out: &mut Vec<f32>) -> bool {
        let Some(bytes) = self.record(slot) else {
            return false;
        };
        out.clear();
        out.extend(
            bytes
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::pager::{CheckpointColl, Pager};
    use crate::storage::segment::Segment;
    use crate::storage::vectors::{Encoding, VectorsBody};

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "veclite-mmap-{}-{name}.veclite",
            std::process::id()
        ))
    }

    fn vectors_seg(coll_id: u32, dim: u32, vecs: &[Vec<f32>]) -> Segment {
        let mut records = Vec::new();
        for v in vecs {
            for f in v {
                records.extend_from_slice(&f.to_le_bytes());
            }
        }
        let body = VectorsBody {
            encoding: Encoding::F32,
            dimension: dim,
            first_slot: 0,
            count: vecs.len() as u64,
            sq_params: None,
            records,
        };
        Segment {
            seg_type: SegmentType::Vectors,
            seg_flags: 0,
            coll_id,
            body: body.encode(),
        }
    }

    #[test]
    fn maps_and_addresses_vectors_by_slot() {
        let path = tmp("addr");
        let _ = std::fs::remove_file(&path);
        let vecs = vec![
            vec![1.0f32, 2.0, 3.0],
            vec![4.0, 5.0, 6.0],
            vec![7.0, 8.0, 9.0],
        ];
        let seg = vectors_seg(0, 3, &vecs);
        {
            let mut p = Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
            p.checkpoint(
                1,
                vec![CheckpointColl {
                    coll_id: 0,
                    name: "c".into(),
                    aliases: vec![],
                    vector_count: 3,
                    tombstone_count: 0,
                    segments: vec![seg],
                    reused: None,
                }],
                Codec::Lz4,
                1001,
            )
            .unwrap_or_else(|e| panic!("{e}"));
        } // pager dropped: lock released, file closed

        let (_p, toc) = Pager::open(&path, false).unwrap_or_else(|e| panic!("{e}"));
        let seg_ref = *toc.collections[0]
            .live_segments
            .iter()
            .find(|s| s.seg_type == SegmentType::Vectors.to_byte())
            .unwrap_or_else(|| panic!("no VECTORS seg"));

        let map = FileMap::map(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(map.len() as u64 >= seg_ref.offset + seg_ref.len);
        let view = map.vectors_view(seg_ref).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(view.count, 3);
        assert_eq!(view.dimension, 3);
        for (slot, want) in vecs.iter().enumerate() {
            assert_eq!(view.f32_record(slot as u64).as_deref(), Some(&want[..]));
        }
        assert!(view.record(3).is_none());
        drop(map); // release the mapping so Windows can remove the file
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn non_vectors_segref_is_rejected() {
        let path = tmp("wrongtype");
        let _ = std::fs::remove_file(&path);
        {
            let _p = Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
        }
        let map = FileMap::map(&path).unwrap_or_else(|e| panic!("{e}"));
        let bogus = SegRef {
            seg_type: SegmentType::Payload.to_byte(),
            offset: 4096,
            len: 32,
        };
        assert!(matches!(
            map.vectors_view(bogus),
            Err(VecLiteError::Corrupt(_))
        ));
        drop(map);
        let _ = std::fs::remove_file(&path);
    }

    /// Write one 2-vector F32 VECTORS segment and return `(path, its SegRef)`.
    fn file_with_vectors(name: &str) -> (std::path::PathBuf, SegRef) {
        let path = tmp(name);
        let _ = std::fs::remove_file(&path);
        let seg = vectors_seg(0, 2, &[vec![1.0f32, 2.0], vec![3.0, 4.0]]);
        {
            let mut p = Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
            p.checkpoint(
                1,
                vec![CheckpointColl {
                    coll_id: 0,
                    name: "c".into(),
                    aliases: vec![],
                    vector_count: 2,
                    tombstone_count: 0,
                    segments: vec![seg],
                    reused: None,
                }],
                Codec::Lz4,
                1001,
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        let (_p, toc) = Pager::open(&path, false).unwrap_or_else(|e| panic!("{e}"));
        let seg_ref = *toc.collections[0]
            .live_segments
            .iter()
            .find(|s| s.seg_type == SegmentType::Vectors.to_byte())
            .unwrap_or_else(|| panic!("no VECTORS seg"));
        (path, seg_ref)
    }

    #[test]
    fn region_accessors_and_out_of_range_slots() {
        let (path, seg_ref) = file_with_vectors("region");
        let map = Arc::new(FileMap::map(&path).unwrap_or_else(|e| panic!("{e}")));
        let region = map
            .vectors_region(seg_ref)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(region.first_slot(), 0);
        assert_eq!(region.count(), 2);
        assert_eq!(region.dimension(), 2);
        assert_eq!(region.byte_len(), 2 * 8); // two f32 records of dim 2

        let mut out = Vec::new();
        assert!(region.read_f32_into(1, &mut out));
        assert_eq!(out, vec![3.0, 4.0]);
        assert!(!region.read_f32_into(2, &mut out)); // past count
        assert!(region.record(9).is_none());

        drop((region, map));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_body_and_truncated_refs_are_corrupt_not_ub() {
        let (path, seg_ref) = file_with_vectors("crcflip");
        // Flip one byte inside the segment body: CRC must catch it.
        {
            use std::io::{Seek, SeekFrom, Write};
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap_or_else(|e| panic!("{e}"));
            f.seek(SeekFrom::Start(seg_ref.offset + 40))
                .unwrap_or_else(|e| panic!("{e}"));
            f.write_all(&[0xFF]).unwrap_or_else(|e| panic!("{e}"));
        }
        let map = Arc::new(FileMap::map(&path).unwrap_or_else(|e| panic!("{e}")));
        let Err(VecLiteError::Corrupt(msg)) = map.vectors_region(seg_ref) else {
            panic!("expected Corrupt on a flipped body byte")
        };
        assert!(msg.contains("crc"), "message was {msg}");

        // A ref whose body runs past EOF is Corrupt, never a panic.
        let past_eof = SegRef {
            seg_type: SegmentType::Vectors.to_byte(),
            offset: map.len() as u64 - 8,
            len: 64,
        };
        assert!(matches!(
            map.vectors_view(past_eof),
            Err(VecLiteError::Corrupt(_))
        ));
        drop(map);
        let _ = std::fs::remove_file(&path);
    }
}
