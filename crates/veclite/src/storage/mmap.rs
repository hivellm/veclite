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

use memmap2::Mmap;

use crate::error::{Result, VecLiteError};
use crate::storage::compression::Codec;
use crate::storage::segment::{SEGMENT_HEADER_SIZE, SegmentType};
use crate::storage::toc::SegRef;
use crate::storage::vectors::VectorsView;

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
        let body_crc32 = u32::from_le_bytes([buf[at + 24], buf[at + 25], buf[at + 26], buf[at + 27]]);
        let body_end = header_end
            .checked_add(stored_len)
            .filter(|&e| e <= buf.len())
            .ok_or_else(|| VecLiteError::Corrupt(format!("{}: body past end of file", loc())))?;
        let body = &buf[header_end..body_end];
        if crc32fast::hash(body) != body_crc32 {
            return Err(VecLiteError::Corrupt(format!("{}: body crc mismatch", loc())));
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::pager::{CheckpointColl, Pager};
    use crate::storage::segment::Segment;
    use crate::storage::vectors::{Encoding, VectorsBody};

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("veclite-mmap-{}-{name}.veclite", std::process::id()))
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
}
