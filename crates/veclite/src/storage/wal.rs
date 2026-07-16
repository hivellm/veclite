//! Write-ahead log (SPEC-003): the `<db>.veclite-wal` sidecar. Every mutating
//! call appends one entry (a batch is one atomic entry); the entry is applied
//! to in-memory state only after the append succeeds, and a checkpoint later
//! moves that state into sealed segments and truncates the WAL (§1).
//!
//! This module is the WAL *codec and file manager*: header, entry framing,
//! durability-aware append, torn-tail-tolerant replay, and truncate. Op bodies
//! are opaque MessagePack blobs here — the recovery layer interprets them per
//! op (SPEC-003 §3).

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::{Result, VecLiteError};
use crate::options::Durability;
use crate::storage::le;

/// `VLWL` — the WAL magic at offset 0.
pub(crate) const WAL_MAGIC: [u8; 4] = *b"VLWL";
pub(crate) const WAL_FORMAT_VERSION: u32 = 1;
/// 16-byte WAL header: magic(4) · format_version(4) · file_uuid_prefix(8).
pub(crate) const WAL_HEADER_SIZE: u64 = 16;
/// Per-entry fixed header: seq(8) · coll_id(4) · op(1) · reserved(3) ·
/// body_len(4) · crc32(4) (SPEC-003 §3). The crc covers the header fields and
/// the body (see `entry_crc`).
const ENTRY_HEADER_SIZE: usize = 24;

/// The eight mutating operations recorded in the WAL (SPEC-003 §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalOp {
    UpsertBatch,
    DeleteBatch,
    CreateColl,
    DropColl,
    Rename,
    Alias,
    VocabUpdate,
    PidxDeclare,
}

impl WalOp {
    pub(crate) fn to_byte(self) -> u8 {
        match self {
            WalOp::UpsertBatch => 1,
            WalOp::DeleteBatch => 2,
            WalOp::CreateColl => 3,
            WalOp::DropColl => 4,
            WalOp::Rename => 5,
            WalOp::Alias => 6,
            WalOp::VocabUpdate => 7,
            WalOp::PidxDeclare => 8,
        }
    }

    pub(crate) fn from_byte(b: u8) -> Result<WalOp> {
        Ok(match b {
            1 => WalOp::UpsertBatch,
            2 => WalOp::DeleteBatch,
            3 => WalOp::CreateColl,
            4 => WalOp::DropColl,
            5 => WalOp::Rename,
            6 => WalOp::Alias,
            7 => WalOp::VocabUpdate,
            8 => WalOp::PidxDeclare,
            other => return Err(VecLiteError::Corrupt(format!("wal: unknown op {other}"))),
        })
    }
}

/// One decoded WAL entry. `body` is the opaque MessagePack op payload.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WalEntry {
    pub(crate) seq: u64,
    pub(crate) coll_id: u32,
    pub(crate) op: WalOp,
    pub(crate) body: Vec<u8>,
}

/// Offset of the 20-byte integrity-protected header prefix within an entry:
/// seq(8) · coll_id(4) · op(1) · reserved(3) · body_len(4). The `crc32` field
/// at bytes [20..24] covers this prefix **and** the body, so a bit flip in any
/// header field (not just the body) is caught on replay (SPEC-003 §3).
const ENTRY_CRC_PREFIX: usize = 20;

/// CRC32 over an entry's header prefix concatenated with its body.
fn entry_crc(header_prefix: &[u8], body: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(header_prefix);
    h.update(body);
    h.finalize()
}

impl WalEntry {
    /// Serialize the entry (24-byte header + body); the CRC covers the header
    /// fields and the body (SPEC-003 §3).
    fn encode(&self) -> Result<Vec<u8>> {
        let body_len = u32::try_from(self.body.len())
            .map_err(|_| VecLiteError::Corrupt("wal: entry body exceeds 4 GiB".to_owned()))?;
        let mut out = Vec::with_capacity(ENTRY_HEADER_SIZE + self.body.len());
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&self.coll_id.to_le_bytes());
        out.push(self.op.to_byte());
        out.extend_from_slice(&[0u8; 3]); // reserved
        out.extend_from_slice(&body_len.to_le_bytes());
        debug_assert_eq!(out.len(), ENTRY_CRC_PREFIX);
        out.extend_from_slice(&entry_crc(&out, &self.body).to_le_bytes());
        out.extend_from_slice(&self.body);
        Ok(out)
    }
}

/// The WAL sidecar file manager.
pub(crate) struct Wal {
    file: File,
    uuid_prefix: [u8; 8],
    /// Next sequence number to assign (starts at 1 after each checkpoint).
    next_seq: u64,
}

/// Result of a replay: the recovered entries plus whether a torn/stale tail was
/// discarded (surfaced to the open path's warning callback).
pub(crate) struct Replay {
    pub(crate) entries: Vec<WalEntry>,
    pub(crate) discarded_tail: bool,
}

impl Wal {
    /// Open the WAL for a database whose file uuid is `uuid`, creating it lazily
    /// with a fresh header if absent. `uuid_prefix` guards against a stale
    /// sidecar copied from another database (WAL-002).
    pub(crate) fn open(path: &Path, uuid: [u8; 16]) -> Result<Wal> {
        let mut uuid_prefix = [0u8; 8];
        uuid_prefix.copy_from_slice(&uuid[..8]);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let len = file.seek(SeekFrom::End(0))?;
        if len < WAL_HEADER_SIZE {
            write_header(&mut file, uuid_prefix)?;
        }
        Ok(Wal {
            file,
            uuid_prefix,
            next_seq: 1,
        })
    }

    /// Append one entry, assigning the next `seq`. fsyncs per durability mode
    /// (WAL-020): `Full` fsyncs every append; `Normal`/`Off` defer to
    /// checkpoint.
    pub(crate) fn append(
        &mut self,
        coll_id: u32,
        op: WalOp,
        body: Vec<u8>,
        durability: Durability,
    ) -> Result<u64> {
        let seq = self.next_seq;
        let entry = WalEntry {
            seq,
            coll_id,
            op,
            body,
        };
        let bytes = entry.encode()?;
        self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&bytes)?;
        if durability == Durability::Full {
            self.file.sync_all()?;
        }
        self.next_seq += 1;
        Ok(seq)
    }

    /// fsync the WAL (used at checkpoint/close for `Normal`/`Off` — WAL-020).
    pub(crate) fn sync(&self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

    /// Replay entries in `seq` order. Stops at the first torn/invalid entry
    /// (bad crc, non-contiguous seq, short read) and discards it and everything
    /// after (WAL-010/011). A uuid-prefix mismatch means a stale sidecar: no
    /// entries are returned and the tail is reported as discarded (WAL-002).
    pub(crate) fn replay(&mut self) -> Result<Replay> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut all = Vec::new();
        self.file.read_to_end(&mut all)?;
        if all.len() < WAL_HEADER_SIZE as usize {
            // Fresh/empty WAL.
            return Ok(Replay {
                entries: Vec::new(),
                discarded_tail: false,
            });
        }
        if all[0..4] != WAL_MAGIC || all[8..16] != self.uuid_prefix {
            // Foreign or corrupt header → ignore the whole WAL (WAL-002).
            return Ok(Replay {
                entries: Vec::new(),
                discarded_tail: true,
            });
        }

        let mut entries = Vec::new();
        let mut at = WAL_HEADER_SIZE as usize;
        let mut expected_seq = 1u64;
        let mut discarded_tail = false;
        while at < all.len() {
            match decode_entry(&all, at, expected_seq) {
                Some((entry, next)) => {
                    expected_seq = entry.seq + 1;
                    at = next;
                    entries.push(entry);
                }
                None => {
                    // Torn tail: discard from here on.
                    discarded_tail = at < all.len();
                    break;
                }
            }
        }
        self.next_seq = expected_seq;
        Ok(Replay {
            entries,
            discarded_tail,
        })
    }

    /// Truncate to the bare 16-byte header and reset `seq` — the last step of a
    /// checkpoint, done only after the main-file header swap fsync (WAL-031/032).
    pub(crate) fn truncate(&mut self) -> Result<()> {
        self.file.set_len(WAL_HEADER_SIZE)?;
        self.file.seek(SeekFrom::Start(0))?;
        write_header(&mut self.file, self.uuid_prefix)?;
        self.file.sync_all()?;
        self.next_seq = 1;
        Ok(())
    }

    /// Current file size in bytes (header + entries).
    pub(crate) fn size(&mut self) -> Result<u64> {
        Ok(self.file.seek(SeekFrom::End(0))?)
    }

    /// True when the WAL holds no entries (only the header).
    pub(crate) fn is_empty(&mut self) -> Result<bool> {
        Ok(self.size()? <= WAL_HEADER_SIZE)
    }
}

fn write_header(file: &mut File, uuid_prefix: [u8; 8]) -> Result<()> {
    let mut header = [0u8; WAL_HEADER_SIZE as usize];
    header[0..4].copy_from_slice(&WAL_MAGIC);
    header[4..8].copy_from_slice(&WAL_FORMAT_VERSION.to_le_bytes());
    header[8..16].copy_from_slice(&uuid_prefix);
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    Ok(())
}

/// Decode one entry at `all[at]`, requiring `seq == expected_seq` and a valid
/// body CRC. Returns `None` on any torn/invalid entry (WAL-010/011).
fn decode_entry(all: &[u8], at: usize, expected_seq: u64) -> Option<(WalEntry, usize)> {
    if at + ENTRY_HEADER_SIZE > all.len() {
        return None;
    }
    let seq = le::u64(all, at, "wal").ok()?;
    if seq != expected_seq {
        return None; // gap / non-monotonic → torn tail (WAL-010)
    }
    let coll_id = le::u32(all, at + 8, "wal").ok()?;
    let op = WalOp::from_byte(all[at + 12]).ok()?;
    let body_len = le::u32(all, at + 16, "wal").ok()? as usize;
    let stored_crc = le::u32(all, at + 20, "wal").ok()?;
    let body_start = at + ENTRY_HEADER_SIZE;
    let body_end = body_start
        .checked_add(body_len)
        .filter(|&e| e <= all.len())?;
    let body = &all[body_start..body_end];
    // The CRC covers the header prefix (seq/coll_id/op/reserved/body_len) plus
    // the body, so a flip in any header field is a torn/corrupt entry (WAL-011).
    if entry_crc(&all[at..at + ENTRY_CRC_PREFIX], body) != stored_crc {
        return None;
    }
    Some((
        WalEntry {
            seq,
            coll_id,
            op,
            body: body.to_vec(),
        },
        body_end,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("veclite-wal-{}-{name}.wal", std::process::id()))
    }

    const UUID: [u8; 16] = [7u8; 16];

    fn fresh(name: &str) -> (std::path::PathBuf, Wal) {
        let path = tmp(name);
        let _ = std::fs::remove_file(&path);
        let wal = Wal::open(&path, UUID).unwrap_or_else(|e| panic!("{e}"));
        (path, wal)
    }

    #[test]
    fn append_replay_round_trip() {
        let (path, mut wal) = fresh("rt");
        wal.append(0, WalOp::CreateColl, b"cfg".to_vec(), Durability::Full)
            .unwrap_or_else(|e| panic!("{e}"));
        wal.append(
            0,
            WalOp::UpsertBatch,
            b"points".to_vec(),
            Durability::Normal,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        wal.append(1, WalOp::DeleteBatch, b"ids".to_vec(), Durability::Off)
            .unwrap_or_else(|e| panic!("{e}"));

        let replay = wal.replay().unwrap_or_else(|e| panic!("{e}"));
        assert!(!replay.discarded_tail);
        assert_eq!(replay.entries.len(), 3);
        assert_eq!(replay.entries[0].op, WalOp::CreateColl);
        assert_eq!(replay.entries[0].seq, 1);
        assert_eq!(replay.entries[2].coll_id, 1);
        assert_eq!(replay.entries[2].body, b"ids");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn torn_tail_is_discarded_prior_entries_kept() {
        let (path, mut wal) = fresh("torn");
        for i in 0..5 {
            wal.append(0, WalOp::UpsertBatch, vec![i as u8; 20], Durability::Full)
                .unwrap_or_else(|e| panic!("{e}"));
        }
        // Corrupt the last few bytes (simulating a torn final entry).
        {
            use std::io::Write;
            let mut f = OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap_or_else(|e| panic!("{e}"));
            let end = f.seek(SeekFrom::End(0)).unwrap_or_else(|e| panic!("{e}"));
            f.seek(SeekFrom::Start(end - 3))
                .unwrap_or_else(|e| panic!("{e}"));
            f.write_all(&[0xFF, 0xFF, 0xFF])
                .unwrap_or_else(|e| panic!("{e}"));
            f.sync_all().unwrap_or_else(|e| panic!("{e}"));
        }
        let replay = wal.replay().unwrap_or_else(|e| panic!("{e}"));
        assert!(replay.discarded_tail);
        assert_eq!(replay.entries.len(), 4); // 5th torn, first 4 intact
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stale_uuid_wal_is_ignored() {
        let (path, mut wal) = fresh("stale");
        wal.append(0, WalOp::UpsertBatch, b"x".to_vec(), Durability::Full)
            .unwrap_or_else(|e| panic!("{e}"));
        // Reopen with a different uuid → stale sidecar, ignored (WAL-002).
        let mut other = Wal::open(&path, [9u8; 16]).unwrap_or_else(|e| panic!("{e}"));
        let replay = other.replay().unwrap_or_else(|e| panic!("{e}"));
        assert!(replay.entries.is_empty());
        assert!(replay.discarded_tail);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sync_flushes_without_error() {
        let (path, mut wal) = fresh("sync");
        wal.append(0, WalOp::UpsertBatch, b"x".to_vec(), Durability::Off)
            .unwrap_or_else(|e| panic!("{e}"));
        wal.sync().unwrap_or_else(|e| panic!("{e}"));
        // The synced entry survives a fresh open of the same file.
        let mut re = Wal::open(&path, UUID).unwrap_or_else(|e| panic!("{e}"));
        let replay = re.replay().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(replay.entries.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn replay_of_file_shorter_than_header_is_fresh_not_discarded() {
        let (path, mut wal) = fresh("short");
        // The header is written by `open`; truncate below WAL_HEADER_SIZE to
        // simulate a file that never got a complete header written.
        {
            let f = OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap_or_else(|e| panic!("{e}"));
            f.set_len(WAL_HEADER_SIZE - 1)
                .unwrap_or_else(|e| panic!("{e}"));
        }
        let replay = wal.replay().unwrap_or_else(|e| panic!("{e}"));
        assert!(replay.entries.is_empty());
        assert!(!replay.discarded_tail);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn truncate_resets_and_persists_across_reopen() {
        let (path, mut wal) = fresh("trunc");
        wal.append(0, WalOp::UpsertBatch, b"a".to_vec(), Durability::Full)
            .unwrap_or_else(|e| panic!("{e}"));
        wal.truncate().unwrap_or_else(|e| panic!("{e}"));
        assert!(wal.is_empty().unwrap_or_else(|e| panic!("{e}")));
        // Reopen: header still valid, no entries.
        let mut re = Wal::open(&path, UUID).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            re.replay()
                .unwrap_or_else(|e| panic!("{e}"))
                .entries
                .is_empty()
        );
        let _ = std::fs::remove_file(&path);
    }
}
