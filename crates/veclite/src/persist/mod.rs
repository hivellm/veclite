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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::error::{Result, VecLiteError};
use crate::options::Durability;
use crate::storage::compression::Codec;
use crate::storage::mmap::FileMap;
use crate::storage::pager::{CheckpointColl, Pager};
use crate::storage::segment::SegmentType;
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
    /// Tombstone ratio (0.0..1.0) that escalates a checkpoint to a vacuum
    /// (STG-072); 0.0 disables auto-vacuum.
    auto_vacuum_threshold: f32,
    /// A read-only database rejects every mutation with `ReadOnly` (STG-062).
    read_only: bool,
    /// Set to skip the close-time checkpoint (a simulated crash, for tests) —
    /// the WAL is then left for recovery to replay. The lock still releases.
    crashed: AtomicBool,
    checkpoint: OnceLock<CheckpointFn>,
}

/// Everything an `open` recovered: the checkpointed collections, the WAL
/// entries to replay on top, and whether a torn/stale WAL tail was discarded.
pub(crate) struct LoadedState {
    /// `(name, coll_id, aliases, loaded)` per collection recovered from the TOC.
    pub(crate) collections: Vec<(String, u32, Vec<String>, seal::LoadedCollection)>,
    pub(crate) replay_entries: Vec<WalEntry>,
    pub(crate) discarded_tail: bool,
}

/// `Persistence::open` knobs, mirroring the relevant `OpenOptions` fields.
pub(crate) struct OpenConfig {
    pub(crate) durability: Durability,
    pub(crate) wal_size_limit: u64,
    pub(crate) auto_vacuum_threshold: f32,
    pub(crate) read_only: bool,
    pub(crate) read_only_ignore_wal: bool,
    /// `None` = auto: mmap a collection whose VECTORS exceed 64 MiB (SPEC-004 §1).
    pub(crate) mmap: Option<bool>,
    /// STG-064 tier split: mapped collections at most this large rebuild HNSW.
    pub(crate) memory_budget: u64,
}

/// Auto-mmap threshold (SPEC-004 §1): collections whose VECTORS segments exceed
/// this stay in the file map instead of being materialized.
const MMAP_AUTO_THRESHOLD: u64 = 64 * 1024 * 1024;

impl Persistence {
    /// Open (or create) the pager + WAL for `path`, take the advisory lock, load
    /// each collection from its checkpointed segments, and read (but do not yet
    /// apply) the WAL. `read_only` takes a shared lock and never replays;
    /// `read_only_ignore_wal` lets it open past a pending WAL (STG-060/062,
    /// WAL-043).
    pub(crate) fn open(path: &Path, cfg: &OpenConfig) -> Result<(Persistence, LoadedState)> {
        let created_epoch_s = now_epoch_s();
        let read_only = cfg.read_only;
        // The pager locks its own handle (STG-060): exclusive for read-write,
        // shared for read-only — before any read, so a conflict is `Locked`.
        let (mut pager, toc) = open_pager(path, created_epoch_s, !read_only)?;
        let uuid = pager.uuid();

        // Reconstruct each collection from its live segments (STG-041 order is
        // already baked into the TOC). VECTORS segments large enough for the
        // mmap tier (STG-004, ADR-0004) stay in the file map; everything else
        // is materialized as before.
        let mut filemap: Option<Arc<FileMap>> = None;
        let mut collections = Vec::with_capacity(toc.collections.len());
        for entry in &toc.collections {
            let mut segments = Vec::with_capacity(entry.live_segments.len());
            let mut vec_refs = Vec::new();
            for seg_ref in &entry.live_segments {
                if seg_ref.seg_type == SegmentType::Vectors.to_byte() {
                    vec_refs.push(*seg_ref);
                } else {
                    segments.push(pager.read_segment(*seg_ref)?);
                }
            }
            let vectors_bytes: u64 = vec_refs.iter().map(|r| r.len).sum();
            let use_mmap =
                !vec_refs.is_empty() && cfg.mmap.unwrap_or(vectors_bytes > MMAP_AUTO_THRESHOLD);
            if use_mmap {
                let map = match &filemap {
                    Some(m) => Arc::clone(m),
                    None => {
                        let m = Arc::new(FileMap::map(pager.path())?);
                        filemap = Some(Arc::clone(&m));
                        m
                    }
                };
                let mut regions = Vec::with_capacity(vec_refs.len());
                for r in &vec_refs {
                    regions.push(map.vectors_region(*r)?);
                }
                let slot_count = regions
                    .iter()
                    .map(|g| g.first_slot() + g.count())
                    .max()
                    .unwrap_or(0);
                let slot_count = usize::try_from(slot_count)
                    .map_err(|_| VecLiteError::Corrupt("load: slot count exceeds usize".into()))?;
                let (options, ids, payloads) = seal::load_based(&segments, slot_count)?;
                // Auto-embed collections re-derive every vector from `_text` on
                // open (their vocabulary is a function of the live corpus), so
                // the map saves nothing — they always materialize.
                if options.embedding_provider.is_none() {
                    let indexed = vectors_bytes <= cfg.memory_budget;
                    collections.push((
                        entry.name.clone(),
                        entry.coll_id,
                        entry.aliases.clone(),
                        seal::LoadedCollection {
                            options,
                            points: Vec::new(),
                            base: Some(seal::LoadedBase {
                                regions,
                                slot_count,
                                ids,
                                payloads,
                                seg_refs: entry.live_segments.clone(),
                                vector_count: entry.vector_count,
                                tombstone_count: entry.tombstone_count,
                                indexed,
                            }),
                        },
                    ));
                    continue;
                }
            }
            for r in &vec_refs {
                segments.push(pager.read_segment(*r)?);
            }
            collections.push((
                entry.name.clone(),
                entry.coll_id,
                entry.aliases.clone(),
                seal::load(&segments)?,
            ));
        }

        let mut wal = Wal::open(&wal_path(path), uuid)?;
        let (replay_entries, discarded_tail) = if read_only {
            // Read-only never replays (WAL-043). A non-empty WAL means there are
            // uncheckpointed writes a reader would miss → refuse unless the
            // caller opted to read the last checkpoint.
            if !wal.is_empty()? && !cfg.read_only_ignore_wal {
                return Err(VecLiteError::WalPending);
            }
            (Vec::new(), false)
        } else {
            let replay = wal.replay()?;
            (replay.entries, replay.discarded_tail)
        };

        Ok((
            Persistence {
                journal: Mutex::new(Journal {
                    pager,
                    wal,
                    generation: toc.generation,
                }),
                uuid,
                durability: cfg.durability,
                wal_size_limit: cfg.wal_size_limit,
                auto_vacuum_threshold: cfg.auto_vacuum_threshold,
                read_only,
                crashed: AtomicBool::new(false),
                checkpoint: OnceLock::new(),
            },
            LoadedState {
                collections,
                replay_entries,
                discarded_tail,
            },
        ))
    }

    pub(crate) fn uuid(&self) -> [u8; 16] {
        self.uuid
    }

    pub(crate) fn durability(&self) -> Durability {
        self.durability
    }

    /// Mark the database as crashed: the close-time checkpoint is skipped, so
    /// the WAL survives for recovery (used to test crash recovery).
    pub(crate) fn mark_crashed(&self) {
        self.crashed.store(true, Ordering::Release);
    }

    pub(crate) fn is_crashed(&self) -> bool {
        self.crashed.load(Ordering::Acquire)
    }

    /// Wire the checkpoint driver (called once, right after the database Arc
    /// exists).
    pub(crate) fn set_checkpoint(&self, f: CheckpointFn) {
        let _ = self.checkpoint.set(f);
    }

    /// Append one mutation to the WAL (SPEC-003 §3). fsync policy per durability
    /// mode is applied inside `Wal::append`.
    pub(crate) fn append(&self, coll_id: u32, op: WalOp, body: Vec<u8>) -> Result<()> {
        if self.read_only {
            return Err(VecLiteError::ReadOnly);
        }
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
        if self.read_only {
            return Ok(()); // nothing to persist; a reader never mutates the file
        }
        let mut j = self.journal.lock();
        j.generation += 1;
        let generation = j.generation;
        let epoch = now_epoch_s();
        j.pager.checkpoint(generation, colls, Codec::Lz4, epoch)?;
        // The pager already fsync'd the header swap; now the WAL is safe to drop.
        j.wal.truncate()?;
        Ok(())
    }

    /// Tombstone ratio that escalates a checkpoint to a vacuum (STG-072).
    pub(crate) fn auto_vacuum_threshold(&self) -> f32 {
        self.auto_vacuum_threshold
    }

    /// Write a compacted standalone copy at `path` with a **fresh** `file_uuid`
    /// (SPEC-002 STG-070). `colls` is the caller's sealed live state; the source
    /// file and WAL are untouched, so the copy never blocks writers. Fails if
    /// `path` already exists (a snapshot never clobbers).
    pub(crate) fn write_snapshot(path: &Path, colls: Vec<CheckpointColl>) -> Result<()> {
        let uuid = *uuid::Uuid::new_v4().as_bytes();
        Pager::create_compacted(path, uuid, 1, colls, Codec::Lz4, now_epoch_s())
    }

    /// Compact the file in place (SPEC-002 STG-071): write a fresh single-
    /// generation file (the source `file_uuid` preserved) holding only `colls`,
    /// atomically swap it onto the live path, then drop the now-redundant WAL.
    /// Crash-safe: a crash before the swap leaves the original file and WAL
    /// intact; a crash after leaves the compacted file, whose uuid still matches
    /// the WAL, so replay stays valid and idempotent (WAL-042). Read-only is a
    /// no-op.
    pub(crate) fn vacuum(&self, colls: Vec<CheckpointColl>) -> Result<()> {
        if self.read_only {
            return Ok(());
        }
        let mut j = self.journal.lock();
        let uuid = j.pager.uuid();
        let temp = vacuum_temp_path(j.pager.path());
        // A leftover temp from an interrupted vacuum is our own artifact.
        let _ = std::fs::remove_file(&temp);
        j.generation += 1;
        let generation = j.generation;
        Pager::create_compacted(&temp, uuid, generation, colls, Codec::Lz4, now_epoch_s())?;
        // Windows-safe close→rename→reopen; readers are served from memory.
        j.pager.replace_with(&temp)?;
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

/// The vacuum scratch path: `<db>.veclite-vac`. The compacted file is written
/// here, then atomically renamed onto the live path (STG-071).
fn vacuum_temp_path(db: &Path) -> std::path::PathBuf {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push("-vac");
    db.with_file_name(name)
}

/// Open an existing `.veclite` file, or create a fresh one with an empty gen-0
/// TOC. Returns the pager and its current TOC.
fn open_pager(
    path: &Path,
    created_epoch_s: u64,
    exclusive: bool,
) -> Result<(Pager, crate::storage::toc::Toc)> {
    if path.exists() {
        Pager::open(path, exclusive)
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

    fn cfg() -> OpenConfig {
        OpenConfig {
            durability: Durability::Normal,
            wal_size_limit: 64 << 20,
            auto_vacuum_threshold: 0.25,
            read_only: false,
            read_only_ignore_wal: false,
            mmap: None,
            memory_budget: crate::options::DEFAULT_MEMORY_BUDGET,
        }
    }

    #[test]
    fn open_fresh_then_reopen() {
        let path = tmp("fresh");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(wal_path(&path));
        {
            let (p, state) = Persistence::open(&path, &cfg()).unwrap_or_else(|e| panic!("{e}"));
            assert!(state.collections.is_empty());
            assert!(state.replay_entries.is_empty());
            let _ = p;
        }
        // Reopen: empty (no collections, clean WAL).
        let (_, state) = Persistence::open(&path, &cfg()).unwrap_or_else(|e| panic!("{e}"));
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
