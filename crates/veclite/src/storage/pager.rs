//! The pager: file creation, read-back, and the root-pointer-swap commit
//! protocol (SPEC-002 §5, STG-050). A checkpoint appends immutable segments,
//! fsyncs, appends a new TOC, fsyncs, then rewrites the 4 KiB header to point
//! at it and fsyncs again. A crash between any two steps leaves the previous
//! header→TOC chain intact (STG-003), because nothing committed is ever
//! overwritten — only the header is rewritten in place, atomically.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::{Result, VecLiteError};
use crate::storage::compression::Codec;
use crate::storage::header::{FLAG_CLEAN_CLOSE, HEADER_SIZE, Header};
// `CheckpointColl` lives in the portable `image` module (it is also the input to
// the wasm in-memory image writer); the pager consumes it here.
use crate::storage::image::CheckpointColl;
use crate::storage::segment::{Segment, codec_for};
use crate::storage::toc::{CollEntry, SegRef, Toc};

/// Owns the open `.veclite` file and tracks the next append offset. The handle
/// is `None` only during the brief close→rename→reopen window of `replace_with`
/// (vacuum); every other method assumes it is present.
pub(crate) struct Pager {
    file: Option<File>,
    uuid: [u8; 16],
    created_epoch_s: u64,
    /// Byte offset where the next appended segment/TOC will start.
    tail: u64,
    /// The `.veclite` path this pager serves (used by `replace_with`).
    path: PathBuf,
}

fn as_usize(v: u64, ctx: &str) -> Result<usize> {
    usize::try_from(v).map_err(|_| VecLiteError::Corrupt(format!("{ctx}: offset exceeds usize")))
}

/// Sibling temp path (`<name>-new`) used while materializing a brand-new
/// database file, mirroring vacuum's `<name>-vac` convention.
fn creation_temp_path(db: &Path) -> PathBuf {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push("-new");
    db.with_file_name(name)
}

/// Read and validate the committed header→TOC chain from a freshly opened file
/// (STG-010/051): decode the header, CRC-check and decode the TOC it points at,
/// and compute the next-append tail. Shared by `open` and `replace_with`.
fn read_committed(file: &mut File) -> Result<(Header, Toc, u64)> {
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
    Ok((header, toc, tail))
}

/// Advisory lock on `file` (STG-060): exclusive for read-write, shared for
/// read-only. On the pager's own handle, so the pager's I/O is unaffected while
/// another process's handle is blocked; a conflict is `Locked`, not a wait.
fn lock_file(file: &std::fs::File, exclusive: bool) -> Result<()> {
    use fs4::fs_std::FileExt;
    // UFCS: std gained inherent `try_lock_*` in 1.89 (a `TryLockError` API)
    // that shadows fs4's; those don't exist on our MSRV 1.85 — pin to fs4.
    let acquired = if exclusive {
        FileExt::try_lock_exclusive(file)?
    } else {
        FileExt::try_lock_shared(file)?
    };
    if acquired {
        Ok(())
    } else {
        Err(VecLiteError::Locked)
    }
}

impl Pager {
    /// Create a brand-new file with a fresh v4 uuid and an initial empty
    /// checkpoint (generation 0). Fails if the file already exists.
    ///
    /// The initial generation is materialized in a sibling temp file and
    /// renamed into place. STG-050's crash-safety argument needs a previous
    /// header→TOC chain to fall back on, and a brand-new file has none: its
    /// TOC is appended before the header is written, so a kill inside the
    /// first checkpoint left a zeroed header and a permanently unopenable
    /// file (caught by the kill-9 harness, `xtask crash`). The rename is
    /// atomic on the same volume, so a crash mid-creation leaves nothing at
    /// `path` at all.
    pub(crate) fn create(path: &Path, created_epoch_s: u64) -> Result<Pager> {
        let temp = creation_temp_path(path);
        // A leftover temp from an interrupted creation is our own artifact.
        let _ = std::fs::remove_file(&temp);
        Self::create_compacted(
            &temp,
            *uuid::Uuid::new_v4().as_bytes(),
            0,
            Vec::new(),
            Codec::Lz4,
            created_epoch_s,
        )?;
        if path.exists() {
            // Preserve create-new semantics: never rename over a database some
            // other process created since the caller's existence check.
            let _ = std::fs::remove_file(&temp);
            return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists).into());
        }
        std::fs::rename(&temp, path)?;
        let (pager, _toc) = Self::open(path, true)?;
        Ok(pager)
    }

    /// Open an existing file read-write: validate the header, then load and
    /// CRC-check the TOC it points at (STG-010/051).
    pub(crate) fn open(path: &Path, exclusive: bool) -> Result<(Pager, Toc)> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        // Lock BEFORE any read (STG-060): another process's exclusive lock must
        // surface as `Locked`, not a mid-read I/O error.
        lock_file(&file, exclusive)?;
        let (header, toc, tail) = read_committed(&mut file)?;
        Ok((
            Pager {
                file: Some(file),
                uuid: header.file_uuid,
                created_epoch_s: header.created_epoch_s,
                tail,
                path: path.to_owned(),
            },
            toc,
        ))
    }

    /// Write a single-generation compacted file at `path` with the given
    /// `uuid`, then close it (used by snapshot with a fresh uuid, and by vacuum
    /// with the source uuid preserved). Fails if `path` already exists.
    pub(crate) fn create_compacted(
        path: &Path,
        uuid: [u8; 16],
        generation: u64,
        colls: Vec<CheckpointColl>,
        codec: Codec,
        epoch: u64,
    ) -> Result<()> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        lock_file(&file, true)?;
        let mut pager = Pager {
            file: Some(file),
            uuid,
            created_epoch_s: epoch,
            tail: HEADER_SIZE as u64,
            path: path.to_owned(),
        };
        pager.checkpoint(generation, colls, codec, epoch)?;
        Ok(()) // pager dropped here: handle closed, advisory lock released
    }

    /// Close this pager's handle, atomically move `replacement` onto our path,
    /// and reopen the committed state (STG-071 vacuum swap). Windows-safe: the
    /// handle and its advisory lock are dropped *before* the rename, since
    /// Windows refuses to replace an open file. In-process readers are served
    /// from memory, so none are invalidated by the swap.
    pub(crate) fn replace_with(&mut self, replacement: &Path) -> Result<()> {
        let orig = self.path.clone();
        // Drop the current handle first (closes it, releases the lock).
        self.file = None;
        std::fs::rename(replacement, &orig)?;
        let mut file = OpenOptions::new().read(true).write(true).open(&orig)?;
        lock_file(&file, true)?;
        let (header, _toc, tail) = read_committed(&mut file)?;
        self.uuid = header.file_uuid;
        self.created_epoch_s = header.created_epoch_s;
        self.tail = tail;
        self.file = Some(file);
        Ok(())
    }

    /// The `.veclite` path this pager serves.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Mutable access to the open handle; `Corrupt` only in the impossible case
    /// of use during the `replace_with` swap window.
    fn file(&mut self) -> Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| VecLiteError::Corrupt("pager file handle is closed".into()))
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
        let start_tail = self.tail;
        let uuid = self.uuid;
        let created_epoch_s = self.created_epoch_s;
        let file = self.file()?;

        // (1) append the new segments.
        file.seek(SeekFrom::Start(start_tail))?;
        let mut cur = start_tail;
        let mut entries = Vec::with_capacity(colls.len());
        for c in colls {
            if let Some(refs) = c.reused {
                // Carry-forward: the committed segments are immutable and still
                // live in this same file — reference them, write nothing.
                debug_assert!(c.segments.is_empty());
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
                continue;
            }
            let mut refs = Vec::with_capacity(c.segments.len());
            for seg in &c.segments {
                let chosen = codec_for(seg.seg_type, codec, seg.body.len());
                let bytes = seg.encode(chosen)?;
                file.write_all(&bytes)?;
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
        file.sync_all()?; // (2) fsync segments

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
        file.write_all(&tbytes)?;
        file.sync_all()?; // (4) fsync TOC

        // (5) rewrite the header to point at the new TOC.
        let mut header = Header::new(uuid, created_epoch_s);
        header.flags = FLAG_CLEAN_CLOSE;
        header.toc_offset = toc_start;
        header.toc_len = tbytes.len() as u64;
        header.toc_crc32 = crc32fast::hash(&tbytes);
        header.modified_epoch_s = modified_epoch_s;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header.encode())?;
        file.sync_all()?; // (6) fsync header

        self.tail = toc_start + tbytes.len() as u64;
        Ok(toc)
    }

    /// Read and decode one segment by its TOC reference (verifies the body
    /// CRC, STG-021).
    pub(crate) fn read_segment(&mut self, seg: SegRef) -> Result<Segment> {
        let mut buf = vec![0u8; as_usize(seg.len, "segment")?];
        let file = self.file()?;
        file.seek(SeekFrom::Start(seg.offset))?;
        file.read_exact(&mut buf)
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
            reused: None,
        }
    }

    #[test]
    fn create_open_empty() {
        let path = tmp("empty");
        let _ = std::fs::remove_file(&path);
        {
            Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
        }
        // No creation temp left behind after a successful create.
        assert!(!creation_temp_path(&path).exists());
        let (_, toc) = Pager::open(&path, true).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(toc.generation, 0);
        assert!(toc.collections.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// A kill mid-creation leaves only the sibling `-new` temp (never a torn
    /// file at the target path); the next create must clean it up and succeed.
    #[test]
    fn create_cleans_a_stale_creation_temp() {
        let path = tmp("stale-temp");
        let _ = std::fs::remove_file(&path);
        let temp = creation_temp_path(&path);
        std::fs::write(&temp, b"torn leftover from a killed creation")
            .unwrap_or_else(|e| panic!("{e}"));
        {
            Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
        }
        assert!(!temp.exists(), "stale creation temp was not cleaned");
        let (_, toc) = Pager::open(&path, true).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(toc.generation, 0);
        let _ = std::fs::remove_file(&path);
    }

    /// Creation must still refuse an existing database rather than rename over
    /// it (the pre-rename `create_new` semantics).
    #[test]
    fn create_refuses_an_existing_path() {
        let path = tmp("already-exists");
        let _ = std::fs::remove_file(&path);
        {
            Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
        }
        match Pager::create(&path, 2000) {
            Ok(_) => panic!("create over an existing database must fail"),
            Err(err) => assert!(
                err.to_string().to_lowercase().contains("exist"),
                "expected already-exists, got: {err}"
            ),
        }
        assert!(!creation_temp_path(&path).exists());
        // The original database is untouched.
        let (_, toc) = Pager::open(&path, true).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(toc.generation, 0);
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
        let (mut p, toc) = Pager::open(&path, true).unwrap_or_else(|e| panic!("{e}"));
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
        drop(p); // release the exclusive lock before reopening the same file
        // The latest open sees generation 3 and its segment reads back.
        let (mut p, toc) = Pager::open(&path, true).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(toc.generation, 3);
        let s = p
            .read_segment(toc.collections[0].live_segments[0])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.body, vec![3u8; 2000]);
        let _ = std::fs::remove_file(&path);
    }
}
