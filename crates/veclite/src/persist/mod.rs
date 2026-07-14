//! Persistence orchestration (SPEC-002 §5 + SPEC-003): turns the in-memory
//! engine into a durable, single-file database. This layer sits above the
//! storage codec (`crate::storage`) and the in-memory engine
//! (`collection`/`database`): it maps runtime state to and from segments
//! (`config`, `seal`), records mutations in the WAL (`wal_body`), and drives
//! open/checkpoint/recovery/close.
//!
//! Native-only, like `storage` — wasm32 has no file storage (CORE-004).

pub(crate) mod config;
pub(crate) mod seal;
pub(crate) mod wal_body;

use std::path::Path;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::error::Result;
use crate::options::Durability;
use crate::storage::compression::Codec;
use crate::storage::pager::{CheckpointColl, Pager};
use crate::storage::wal::{Wal, WalEntry, WalOp};

/// Seconds since the Unix epoch, saturating on a pre-1970 clock (never panics).
pub(crate) fn now_epoch_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The mutable journal: the main-file pager and the WAL, plus the running
/// generation. Guarded by one mutex so the commit sequence (SPEC-002 §5) and
/// WAL appends never interleave.
struct Journal {
    pager: Pager,
    wal: Wal,
    generation: u64,
}

/// A checkpoint callback, wired after the database is constructed so the
/// WAL-size trigger (WAL-030) can drive `Database::checkpoint` without this
/// layer depending on the database type.
type CheckpointFn = Box<dyn Fn() -> Result<()> + Send + Sync>;

/// The persistence state shared by a file-backed database and its collections.
pub(crate) struct Persistence {
    journal: Mutex<Journal>,
    uuid: [u8; 16],
    durability: Durability,
    wal_size_limit: u64,
    checkpoint: OnceLock<CheckpointFn>,
}

/// Everything an `open` recovered: the checkpointed collections, the WAL
/// entries to replay on top, and whether a torn/stale WAL tail was discarded.
pub(crate) struct LoadedState {
    pub(crate) collections: Vec<(String, u32, seal::LoadedCollection)>,
    pub(crate) replay_entries: Vec<WalEntry>,
    pub(crate) discarded_tail: bool,
}

impl Persistence {
    /// Open (or create) the pager + WAL for `path`, load each collection from
    /// its checkpointed segments, and read (but do not yet apply) the WAL.
    pub(crate) fn open(
        path: &Path,
        durability: Durability,
        wal_size_limit: u64,
    ) -> Result<(Persistence, LoadedState)> {
        let created_epoch_s = now_epoch_s();
        let (mut pager, toc) = open_pager(path, created_epoch_s)?;
        let uuid = pager.uuid();

        // Reconstruct each collection from its live segments (STG-041 order is
        // already baked into the TOC).
        let mut collections = Vec::with_capacity(toc.collections.len());
        for entry in &toc.collections {
            let mut segments = Vec::with_capacity(entry.live_segments.len());
            for seg_ref in &entry.live_segments {
                segments.push(pager.read_segment(*seg_ref)?);
            }
            collections.push((entry.name.clone(), entry.coll_id, seal::load(&segments)?));
        }

        let mut wal = Wal::open(&wal_path(path), uuid)?;
        let replay = wal.replay()?;
        Ok((
            Persistence {
                journal: Mutex::new(Journal {
                    pager,
                    wal,
                    generation: toc.generation,
                }),
                uuid,
                durability,
                wal_size_limit,
                checkpoint: OnceLock::new(),
            },
            LoadedState {
                collections,
                replay_entries: replay.entries,
                discarded_tail: replay.discarded_tail,
            },
        ))
    }

    pub(crate) fn uuid(&self) -> [u8; 16] {
        self.uuid
    }

    pub(crate) fn durability(&self) -> Durability {
        self.durability
    }

    /// Wire the checkpoint driver (called once, right after the database Arc
    /// exists).
    pub(crate) fn set_checkpoint(&self, f: CheckpointFn) {
        let _ = self.checkpoint.set(f);
    }

    /// Append one mutation to the WAL (SPEC-003 §3). fsync policy per durability
    /// mode is applied inside `Wal::append`.
    pub(crate) fn append(&self, coll_id: u32, op: WalOp, body: Vec<u8>) -> Result<()> {
        let mut j = self.journal.lock();
        j.wal.append(coll_id, op, body, self.durability)?;
        Ok(())
    }

    /// After a write: if the WAL crossed the size threshold, drive a checkpoint
    /// on the calling thread (WAL-030a). No-op until the checkpoint driver is
    /// wired.
    pub(crate) fn after_write(&self) -> Result<()> {
        let over = {
            let mut j = self.journal.lock();
            j.wal.size()? >= self.wal_size_limit
        };
        if over {
            if let Some(f) = self.checkpoint.get() {
                f()?;
            }
        }
        Ok(())
    }

    /// Run the commit protocol for `colls` then truncate the WAL (WAL-031):
    /// seal → SPEC-002 §5 commit → fsync WAL (Normal/Off) → truncate. The WAL is
    /// truncated only after the header-swap fsync, so a crash recovers to the
    /// pre- or post-checkpoint state, never between (WAL-032).
    pub(crate) fn commit(&self, colls: Vec<CheckpointColl>) -> Result<()> {
        let mut j = self.journal.lock();
        j.generation += 1;
        let generation = j.generation;
        let epoch = now_epoch_s();
        j.pager.checkpoint(generation, colls, Codec::Lz4, epoch)?;
        // The pager already fsync'd the header swap; now the WAL is safe to drop.
        j.wal.truncate()?;
        Ok(())
    }
}

impl crate::collection::WalSink for Persistence {
    fn log(&self, coll_id: u32, op: u8, body: Vec<u8>) -> Result<()> {
        self.append(coll_id, WalOp::from_byte(op)?, body)
    }
    fn after_write(&self) -> Result<()> {
        Persistence::after_write(self)
    }
}

/// The WAL sidecar path: `<db>.veclite-wal` (WAL-001).
fn wal_path(db: &Path) -> std::path::PathBuf {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push("-wal");
    db.with_file_name(name)
}

/// Open an existing `.veclite` file, or create a fresh one with an empty gen-0
/// TOC. Returns the pager and its current TOC.
fn open_pager(path: &Path, created_epoch_s: u64) -> Result<(Pager, crate::storage::toc::Toc)> {
    if path.exists() {
        Pager::open(path)
    } else {
        let pager = Pager::create(path, created_epoch_s)?;
        let toc = crate::storage::toc::Toc {
            generation: 0,
            collections: Vec::new(),
            free_tail_offset: 0,
        };
        Ok((pager, toc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "veclite-persist-{}-{name}.veclite",
            std::process::id()
        ))
    }

    #[test]
    fn open_fresh_then_reopen() {
        let path = tmp("fresh");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(wal_path(&path));
        {
            let (p, state) = Persistence::open(&path, Durability::Normal, 64 << 20)
                .unwrap_or_else(|e| panic!("{e}"));
            assert!(state.collections.is_empty());
            assert!(state.replay_entries.is_empty());
            let _ = p;
        }
        // Reopen: empty (no collections, clean WAL).
        let (_, state) = Persistence::open(&path, Durability::Normal, 64 << 20)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(state.collections.is_empty());
        assert!(state.replay_entries.is_empty());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(wal_path(&path));
    }

    #[test]
    fn wal_path_is_sidecar() {
        assert_eq!(
            wal_path(Path::new("/tmp/db.veclite")),
            Path::new("/tmp/db.veclite-wal")
        );
    }
}
