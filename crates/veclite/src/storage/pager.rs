//! The pager: file creation, read-back, and the root-pointer-swap commit
//! protocol (SPEC-002 §5, STG-050). A checkpoint appends immutable segments,
//! fsyncs, appends a new TOC, fsyncs, then rewrites the 4 KiB header to point
//! at it and fsyncs again. A crash between any two steps leaves the previous
//! header→TOC chain intact (STG-003), because nothing committed is ever
//! overwritten — only the header is rewritten in place, atomically.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::{Result, VecLiteError};
use crate::storage::compression::Codec;
use crate::storage::header::{FLAG_CLEAN_CLOSE, HEADER_SIZE, Header};
use crate::storage::segment::{Segment, codec_for};
use crate::storage::toc::{CollEntry, SegRef, Toc};

/// One collection's contribution to a checkpoint: its metadata plus the new
/// segments to append. The pager assigns offsets and builds the TOC entry.
pub(crate) struct CheckpointColl {
    pub(crate) coll_id: u32,
    pub(crate) name: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) vector_count: u64,
    pub(crate) tombstone_count: u64,
    pub(crate) segments: Vec<Segment>,
}

/// Owns the open `.veclite` file and tracks the next append offset.
pub(crate) struct Pager {
    file: File,
    uuid: [u8; 16],
    created_epoch_s: u64,
    /// Byte offset where the next appended segment/TOC will start.
    tail: u64,
}

fn as_usize(v: u64, ctx: &str) -> Result<usize> {
    usize::try_from(v).map_err(|_| VecLiteError::Corrupt(format!("{ctx}: offset exceeds usize")))
}

impl Pager {
    /// Create a brand-new file with a fresh v4 uuid and an initial empty
    /// checkpoint (generation 0). Fails if the file already exists.
    pub(crate) fn create(path: &Path, created_epoch_s: u64) -> Result<Pager> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut pager = Pager {
            file,
            uuid: *uuid::Uuid::new_v4().as_bytes(),
            created_epoch_s,
            tail: HEADER_SIZE as u64,
        };
        pager.checkpoint(0, Vec::new(), Codec::Lz4, created_epoch_s)?;
        Ok(pager)
    }

    /// Open an existing file read-write: validate the header, then load and
    /// CRC-check the TOC it points at (STG-010/051).
    pub(crate) fn open(path: &Path) -> Result<(Pager, Toc)> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut hbuf = [0u8; HEADER_SIZE];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut hbuf)
            .map_err(|_| VecLiteError::Corrupt("header: file shorter than 4 KiB".into()))?;
        let header = Header::decode(&hbuf)?;

        let mut tbuf = vec![0u8; as_usize(header.toc_len, "toc")?];
        file.seek(SeekFrom::Start(header.toc_offset))?;
        file.read_exact(&mut tbuf)
            .map_err(|_| VecLiteError::Corrupt("toc: truncated".into()))?;
        if crc32fast::hash(&tbuf) != header.toc_crc32 {
            return Err(VecLiteError::Corrupt("toc: crc mismatch".into()));
        }
        let toc = Toc::decode(&tbuf)?;
        let tail = header.toc_offset + header.toc_len;
        Ok((
            Pager {
                file,
                uuid: header.file_uuid,
                created_epoch_s: header.created_epoch_s,
                tail,
            },
            toc,
        ))
    }

    pub(crate) fn uuid(&self) -> [u8; 16] {
        self.uuid
    }

    /// Run the STG-050 commit sequence and return the committed TOC. `codec` is
    /// the requested body compression (per-segment policy applied by
    /// `codec_for`); `generation` must exceed the previous one (monotonic).
    pub(crate) fn checkpoint(
        &mut self,
        generation: u64,
        colls: Vec<CheckpointColl>,
        codec: Codec,
        modified_epoch_s: u64,
    ) -> Result<Toc> {
        // (1) append the new segments.
        self.file.seek(SeekFrom::Start(self.tail))?;
        let mut cur = self.tail;
        let mut entries = Vec::with_capacity(colls.len());
        for c in colls {
            let mut refs = Vec::with_capacity(c.segments.len());
            for seg in &c.segments {
                let chosen = codec_for(seg.seg_type, codec, seg.body.len());
                let bytes = seg.encode(chosen)?;
                self.file.write_all(&bytes)?;
                refs.push(SegRef {
                    seg_type: seg.seg_type.to_byte(),
                    offset: cur,
                    len: bytes.len() as u64,
                });
                cur += bytes.len() as u64;
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
        self.file.sync_all()?; // (2) fsync segments

        // (3) append the new TOC. `free_tail_offset` records where this TOC
        // begins; the authoritative next-append is `toc_offset + toc_len` from
        // the header, recomputed on open.
        let toc_start = cur;
        let toc = Toc {
            generation,
            collections: entries,
            free_tail_offset: toc_start,
        };
        let tbytes = toc.encode()?;
        self.file.write_all(&tbytes)?;
        self.file.sync_all()?; // (4) fsync TOC

        // (5) rewrite the header to point at the new TOC.
        let mut header = Header::new(self.uuid, self.created_epoch_s);
        header.flags = FLAG_CLEAN_CLOSE;
        header.toc_offset = toc_start;
        header.toc_len = tbytes.len() as u64;
        header.toc_crc32 = crc32fast::hash(&tbytes);
        header.modified_epoch_s = modified_epoch_s;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&header.encode())?;
        self.file.sync_all()?; // (6) fsync header

        self.tail = toc_start + tbytes.len() as u64;
        Ok(toc)
    }

    /// Read and decode one segment by its TOC reference (verifies the body
    /// CRC, STG-021).
    pub(crate) fn read_segment(&mut self, seg: SegRef) -> Result<Segment> {
        let mut buf = vec![0u8; as_usize(seg.len, "segment")?];
        self.file.seek(SeekFrom::Start(seg.offset))?;
        self.file
            .read_exact(&mut buf)
            .map_err(|_| VecLiteError::Corrupt(format!("segment@{}: truncated", seg.offset)))?;
        Ok(Segment::read(&buf, 0, seg.offset)?.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::segment::SegmentType;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "veclite-pager-{}-{name}.veclite",
            std::process::id()
        ))
    }

    fn seg(t: SegmentType, coll_id: u32, body: Vec<u8>) -> Segment {
        Segment {
            seg_type: t,
            seg_flags: 0,
            coll_id,
            body,
        }
    }

    fn coll(id: u32, segs: Vec<Segment>) -> CheckpointColl {
        CheckpointColl {
            coll_id: id,
            name: format!("c{id}"),
            aliases: vec![],
            vector_count: segs.len() as u64,
            tombstone_count: 0,
            segments: segs,
        }
    }

    #[test]
    fn create_open_empty() {
        let path = tmp("empty");
        let _ = std::fs::remove_file(&path);
        {
            Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
        }
        let (_, toc) = Pager::open(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(toc.generation, 0);
        assert!(toc.collections.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn checkpoint_round_trip_and_generations() {
        let path = tmp("rt");
        let _ = std::fs::remove_file(&path);
        let big: Vec<u8> = (0..5000u32).flat_map(|i| (i % 11).to_le_bytes()).collect();
        {
            let mut p = Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
            p.checkpoint(
                1,
                vec![coll(
                    0,
                    vec![
                        seg(SegmentType::Config, 0, b"cfg".to_vec()),
                        seg(SegmentType::Vectors, 0, big.clone()),
                    ],
                )],
                Codec::Lz4,
                1001,
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        let (mut p, toc) = Pager::open(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(toc.generation, 1);
        assert_eq!(toc.collections.len(), 1);
        let entry = &toc.collections[0];
        // Replay order: CONFIG before VECTORS.
        assert_eq!(
            entry.live_segments[0].seg_type,
            SegmentType::Config.to_byte()
        );
        assert_eq!(
            entry.live_segments[1].seg_type,
            SegmentType::Vectors.to_byte()
        );
        // Read the VECTORS segment back — uncompressed (STG-031) and intact.
        let v = p
            .read_segment(entry.live_segments[1])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.body, big);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tail_grows_across_checkpoints_no_overwrite() {
        let path = tmp("grow");
        let _ = std::fs::remove_file(&path);
        let mut p = Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
        for g in 1..=3u64 {
            p.checkpoint(
                g,
                vec![coll(
                    0,
                    vec![seg(SegmentType::Payload, 0, vec![g as u8; 2000])],
                )],
                Codec::Lz4,
                1000 + g,
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        // The latest open sees generation 3 and its segment reads back.
        let (mut p, toc) = Pager::open(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(toc.generation, 3);
        let s = p
            .read_segment(toc.collections[0].live_segments[0])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.body, vec![3u8; 2000]);
        let _ = std::fs::remove_file(&path);
    }
}
