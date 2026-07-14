//! Database handle and collection registry (SPEC-001 §4, SPEC-004 §1–2).
//!
//! Concurrency model (CORE-051): collection lookups are lock-free reads on a
//! `DashMap`; the rare registry mutations (create/delete/rename) serialize on
//! one mutex so two-key operations like rename stay atomic.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dashmap::DashMap;
use parking_lot::Mutex;

use crate::collection::{Collection, CollectionInner, WalSink};
use crate::error::{Result, VecLiteError};
use crate::options::CollectionOptions;
use crate::point::validate_collection_name;

#[cfg(not(target_arch = "wasm32"))]
use crate::options::OpenOptions;
#[cfg(not(target_arch = "wasm32"))]
use crate::persist::{Persistence, config, now_epoch_s, seal, wal_body};
#[cfg(not(target_arch = "wasm32"))]
use crate::point::Point;
#[cfg(not(target_arch = "wasm32"))]
use crate::storage::pager::CheckpointColl;
#[cfg(not(target_arch = "wasm32"))]
use crate::storage::wal::{WalEntry, WalOp};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// Maximum collection dimension (SPEC-002 §8 limits).
const MAX_DIMENSION: usize = 65_536;

struct DatabaseInner {
    collections: DashMap<String, Arc<CollectionInner>>,
    /// Serializes create/delete/rename (registry-level write, CORE-051).
    registry: Mutex<()>,
    /// Next collection id to assign (stamped into WAL entries + CONFIG).
    next_coll_id: AtomicU32,
    /// The shared journal (pager + WAL) for a file-backed database; `None` for
    /// `memory()`. Never set on wasm32.
    #[cfg(not(target_arch = "wasm32"))]
    persistence: Option<Arc<Persistence>>,
}

impl DatabaseInner {
    /// The WAL sink to hand new collections: the shared persistence as a
    /// `dyn WalSink`, or `None` for a memory database.
    fn sink(&self) -> Option<Arc<dyn WalSink>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.persistence
                .as_ref()
                .map(|p| Arc::clone(p) as Arc<dyn WalSink>)
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }
}

/// Locate a live collection handle by its registry id (used by WAL replay).
#[cfg(not(target_arch = "wasm32"))]
fn collection_by_id(inner: &Arc<DatabaseInner>, coll_id: u32) -> Option<Collection> {
    inner
        .collections
        .iter()
        .find(|e| e.value().coll_id == coll_id)
        .map(|e| Collection {
            inner: Arc::clone(e.value()),
        })
}

/// Build a collection from loaded/created state and register it (no WAL log —
/// this is the load/replay path).
#[cfg(not(target_arch = "wasm32"))]
fn install_collection(
    inner: &Arc<DatabaseInner>,
    persistence: &Arc<Persistence>,
    name: String,
    coll_id: u32,
    loaded: seal::LoadedCollection,
) -> Result<()> {
    let sink: Arc<dyn WalSink> = Arc::clone(persistence) as Arc<dyn WalSink>;
    let cinner = Arc::new(CollectionInner::with_capacity(
        name.clone(),
        loaded.options,
        coll_id,
        Some(sink),
        loaded.points.len(),
    )?);
    let handle = Collection {
        inner: Arc::clone(&cinner),
    };
    let points: Vec<Point> = loaded
        .points
        .into_iter()
        .map(|(id, vector, payload)| Point {
            id,
            vector,
            sparse: None,
            payload,
        })
        .collect();
    handle.replay_upsert(points)?;
    inner.collections.insert(name, cinner);
    Ok(())
}

/// Apply one recovered WAL entry to the in-memory registry (SPEC-003 §6).
/// Unknown/not-yet-implemented ops (alias, vocab, pidx) are skipped.
#[cfg(not(target_arch = "wasm32"))]
fn apply_wal_entry(
    inner: &Arc<DatabaseInner>,
    persistence: &Arc<Persistence>,
    entry: WalEntry,
) -> Result<()> {
    match entry.op {
        WalOp::CreateColl => {
            // Idempotent: an already-present coll_id is a no-op (WAL-042).
            if collection_by_id(inner, entry.coll_id).is_some() {
                return Ok(());
            }
            let body: wal_body::CreateColl = wal_body::decode(&entry.body, "create")?;
            let options = config::from_stored(&body.config)?;
            install_collection(
                inner,
                persistence,
                body.name,
                entry.coll_id,
                seal::LoadedCollection {
                    options,
                    points: Vec::new(),
                },
            )?;
        }
        WalOp::DropColl => {
            let name = inner
                .collections
                .iter()
                .find(|e| e.value().coll_id == entry.coll_id)
                .map(|e| e.key().clone());
            if let Some(name) = name {
                if let Some((_, ci)) = inner.collections.remove(&name) {
                    ci.deleted.store(true, Ordering::Release);
                }
            }
        }
        WalOp::Rename => {
            let body: wal_body::Rename = wal_body::decode(&entry.body, "rename")?;
            let name = inner
                .collections
                .iter()
                .find(|e| e.value().coll_id == entry.coll_id)
                .map(|e| e.key().clone());
            if let Some(name) = name {
                if let Some((_, ci)) = inner.collections.remove(&name) {
                    *ci.name.write() = body.new_name.clone();
                    inner.collections.insert(body.new_name, ci);
                }
            }
        }
        WalOp::UpsertBatch => {
            let points: Vec<Point> = wal_body::decode(&entry.body, "upsert")?;
            if let Some(c) = collection_by_id(inner, entry.coll_id) {
                c.replay_upsert(points)?;
            }
        }
        WalOp::DeleteBatch => {
            let ids: Vec<String> = wal_body::decode(&entry.body, "delete")?;
            if let Some(c) = collection_by_id(inner, entry.coll_id) {
                c.replay_delete(&ids);
            }
        }
        WalOp::Alias | WalOp::VocabUpdate | WalOp::PidxDeclare => {}
    }
    Ok(())
}

/// Seal every live collection's current state into `CheckpointColl`s — the
/// input shared by checkpoint, vacuum, and snapshot.
#[cfg(not(target_arch = "wasm32"))]
fn sealed_live_collections(inner: &Arc<DatabaseInner>) -> Result<Vec<CheckpointColl>> {
    let epoch = now_epoch_s();
    let mut colls = Vec::new();
    for entry in inner.collections.iter() {
        let ci = entry.value();
        if ci.deleted.load(Ordering::Acquire) {
            continue;
        }
        let handle = Collection {
            inner: Arc::clone(ci),
        };
        let live = handle.live_points();
        let name = ci.name.read().clone();
        colls.push(seal::seal(
            ci.coll_id,
            name,
            Vec::new(),
            &ci.config,
            &live,
            epoch,
        )?);
    }
    Ok(colls)
}

/// Seal every live collection and run the commit protocol, then truncate the
/// WAL (SPEC-002 §5 / WAL-031). No-op for a memory database.
#[cfg(not(target_arch = "wasm32"))]
fn checkpoint_inner(inner: &Arc<DatabaseInner>) -> Result<()> {
    let Some(persistence) = &inner.persistence else {
        return Ok(());
    };
    persistence.commit(sealed_live_collections(inner)?)
}

/// Checkpoint, then escalate to a vacuum when a collection has churned past the
/// auto-vacuum threshold (SPEC-002 STG-072). Drives the WAL-size trigger and
/// the public `checkpoint()`; the Drop path uses the plain `checkpoint_inner`.
#[cfg(not(target_arch = "wasm32"))]
fn checkpoint_and_maybe_vacuum(inner: &Arc<DatabaseInner>) -> Result<()> {
    checkpoint_inner(inner)?;
    if should_auto_vacuum(inner) {
        vacuum_inner(inner)?;
    }
    Ok(())
}

/// Whether any live collection's tombstone ratio has reached the auto-vacuum
/// threshold (STG-072). False without persistence or when the threshold is
/// disabled (`<= 0`).
#[cfg(not(target_arch = "wasm32"))]
fn should_auto_vacuum(inner: &Arc<DatabaseInner>) -> bool {
    let Some(p) = &inner.persistence else {
        return false;
    };
    let threshold = p.auto_vacuum_threshold();
    if threshold <= 0.0 {
        return false;
    }
    inner.collections.iter().any(|e| {
        let ci = e.value();
        !ci.deleted.load(Ordering::Acquire)
            && Collection {
                inner: Arc::clone(ci),
            }
            .tombstone_ratio()
                >= threshold
    })
}

/// Reclaim dead space (SPEC-002 STG-071): compact every live collection in
/// memory (dropping tombstoned slots), then rewrite the file to a fresh
/// compacted generation and shrink it. No-op for a memory database.
#[cfg(not(target_arch = "wasm32"))]
fn vacuum_inner(inner: &Arc<DatabaseInner>) -> Result<()> {
    let Some(persistence) = &inner.persistence else {
        return Ok(());
    };
    for entry in inner.collections.iter() {
        let ci = entry.value();
        if ci.deleted.load(Ordering::Acquire) {
            continue;
        }
        Collection {
            inner: Arc::clone(ci),
        }
        .compact()?;
    }
    persistence.vacuum(sealed_live_collections(inner)?)
}

/// Database handle. Cheap to clone; `Send + Sync` (CORE-050). Multiple
/// databases in one process are fully independent — no global state.
///
/// ```
/// use veclite::{CollectionOptions, Metric, Point, VecLite};
///
/// let db = VecLite::memory();
/// let docs = db.create_collection("docs", CollectionOptions::new(3, Metric::Cosine))?;
/// docs.upsert(Point::new("id-1", vec![0.1, 0.2, 0.3]))?;
/// assert_eq!(docs.len(), 1);
/// # Ok::<(), veclite::VecLiteError>(())
/// ```
#[derive(Clone)]
pub struct VecLite {
    inner: Arc<DatabaseInner>,
}

impl VecLite {
    /// Ephemeral in-memory database: no file, no WAL, identical API (FR-02).
    pub fn memory() -> Self {
        VecLite {
            inner: Arc::new(DatabaseInner {
                collections: DashMap::new(),
                registry: Mutex::new(()),
                next_coll_id: AtomicU32::new(1),
                #[cfg(not(target_arch = "wasm32"))]
                persistence: None,
            }),
        }
    }

    /// Open (or create) a durable single-file database at `path` with default
    /// options (SPEC-004 §1). On a non-clean previous close, the WAL is replayed
    /// on top of the last checkpoint (SPEC-003 §6).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, OpenOptions::new())
    }

    /// [`open`](Self::open) with tuned [`OpenOptions`] (durability, WAL size
    /// limit).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_with(path: impl AsRef<Path>, options: OpenOptions) -> Result<Self> {
        let path = path.as_ref();
        let (persistence, state) = Persistence::open(
            path,
            options.durability,
            options.wal_size_limit,
            options.auto_vacuum_threshold,
            options.read_only,
            options.read_only_ignore_wal,
        )?;
        let persistence = Arc::new(persistence);
        let inner = Arc::new(DatabaseInner {
            collections: DashMap::new(),
            registry: Mutex::new(()),
            next_coll_id: AtomicU32::new(1),
            persistence: Some(Arc::clone(&persistence)),
        });

        let mut max_coll_id = 0u32;
        for (name, coll_id, loaded) in state.collections {
            install_collection(&inner, &persistence, name, coll_id, loaded)?;
            max_coll_id = max_coll_id.max(coll_id);
        }
        for entry in state.replay_entries {
            max_coll_id = max_coll_id.max(entry.coll_id);
            apply_wal_entry(&inner, &persistence, entry)?;
        }
        inner.next_coll_id.store(max_coll_id + 1, Ordering::Relaxed);

        // Wire the WAL-size checkpoint trigger (WAL-030a) with a weak ref so it
        // never keeps the database alive.
        let weak = Arc::downgrade(&inner);
        persistence.set_checkpoint(Box::new(move || {
            weak.upgrade()
                .map_or(Ok(()), |d| checkpoint_and_maybe_vacuum(&d))
        }));

        if state.discarded_tail {
            eprintln!("veclite: ignored a torn or stale WAL tail (WAL-002/011)");
        }
        Ok(VecLite { inner })
    }

    /// Force a checkpoint now: seal in-memory state into segments and truncate
    /// the WAL (WAL-030b). Escalates to a vacuum when a collection has churned
    /// past the auto-vacuum threshold (STG-072). No-op for a memory database.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn checkpoint(&self) -> Result<()> {
        checkpoint_and_maybe_vacuum(&self.inner)
    }

    /// Write a compacted, standalone point-in-time copy of the database to
    /// `path` (SPEC-002 STG-070). The copy has a fresh `file_uuid`, drops dead
    /// space and tombstoned slots, and opens independently. The live database
    /// (and its writers) are not blocked by the copy. `path` must not already
    /// exist. Works for both file-backed and in-memory databases.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn snapshot(&self, path: impl AsRef<Path>) -> Result<()> {
        // Flush acked state first (STG-070 "run a checkpoint"); a memory
        // database has nothing to flush.
        checkpoint_inner(&self.inner)?;
        Persistence::write_snapshot(path.as_ref(), sealed_live_collections(&self.inner)?)
    }

    /// Reclaim dead space in place (SPEC-002 STG-071): compact live data and
    /// rewrite the file to a fresh compacted generation, shrinking it. Also
    /// runs automatically when a collection's tombstone ratio crosses the
    /// `auto_vacuum_threshold` (STG-072). No-op for a memory database.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn vacuum(&self) -> Result<()> {
        vacuum_inner(&self.inner)
    }

    /// Test hook: mark the database crashed and drop it, so the close-time
    /// checkpoint is skipped and the WAL survives for the next `open` to
    /// replay. The advisory lock is still released. Not part of the stable API.
    #[doc(hidden)]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn __test_simulate_crash(self) {
        if let Some(p) = &self.inner.persistence {
            p.mark_crashed();
        }
    }

    /// Create a collection; `AlreadyExists` when the name is taken
    /// (CORE-020).
    pub fn create_collection(&self, name: &str, options: CollectionOptions) -> Result<Collection> {
        validate_collection_name(name)?;
        if options.dimension == 0 || options.dimension > MAX_DIMENSION {
            return Err(VecLiteError::InvalidArgument(format!(
                "dimension must be in 1..={MAX_DIMENSION}, got {}",
                options.dimension
            )));
        }
        let _guard = self.inner.registry.lock();
        if self.inner.collections.contains_key(name) {
            return Err(VecLiteError::AlreadyExists(name.to_owned()));
        }
        let coll_id = self.inner.next_coll_id.fetch_add(1, Ordering::Relaxed);
        // Log CREATE_COLL before installing, so a crash replays the creation.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = &self.inner.persistence {
            let body = wal_body::encode(&wal_body::CreateColl {
                name: name.to_owned(),
                aliases: Vec::new(),
                config: config::to_stored(&options, now_epoch_s()),
            })?;
            p.append(coll_id, WalOp::CreateColl, body)?;
        }
        let inner = Arc::new(CollectionInner::new(
            name.to_owned(),
            options,
            coll_id,
            self.inner.sink(),
        )?);
        self.inner
            .collections
            .insert(name.to_owned(), Arc::clone(&inner));
        Ok(Collection { inner })
    }

    /// Handle to an existing collection (lock-free lookup, CORE-051).
    pub fn collection(&self, name: &str) -> Result<Collection> {
        match self.inner.collections.get(name) {
            Some(entry) => Ok(Collection {
                inner: Arc::clone(entry.value()),
            }),
            None => Err(VecLiteError::CollectionNotFound(name.to_owned())),
        }
    }

    /// Drop a collection; open handles fail with `CollectionNotFound` from
    /// then on (CORE-021).
    pub fn delete_collection(&self, name: &str) -> Result<()> {
        let _guard = self.inner.registry.lock();
        let coll_id = self
            .inner
            .collections
            .get(name)
            .map(|e| e.coll_id)
            .ok_or_else(|| VecLiteError::CollectionNotFound(name.to_owned()))?;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = &self.inner.persistence {
            p.append(coll_id, WalOp::DropColl, Vec::new())?;
        }
        #[cfg(target_arch = "wasm32")]
        let _ = coll_id;
        if let Some((_, inner)) = self.inner.collections.remove(name) {
            inner.deleted.store(true, Ordering::Release);
        }
        Ok(())
    }

    /// Rename a collection. Metadata-only, O(1) in vector count (CORE-022);
    /// existing handles keep working under the new name.
    pub fn rename_collection(&self, from: &str, to: &str) -> Result<()> {
        validate_collection_name(to)?;
        let _guard = self.inner.registry.lock();
        if self.inner.collections.contains_key(to) {
            return Err(VecLiteError::AlreadyExists(to.to_owned()));
        }
        let coll_id = self
            .inner
            .collections
            .get(from)
            .map(|e| e.coll_id)
            .ok_or_else(|| VecLiteError::CollectionNotFound(from.to_owned()))?;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = &self.inner.persistence {
            let body = wal_body::encode(&wal_body::Rename {
                new_name: to.to_owned(),
            })?;
            p.append(coll_id, WalOp::Rename, body)?;
        }
        #[cfg(target_arch = "wasm32")]
        let _ = coll_id;
        if let Some((_, inner)) = self.inner.collections.remove(from) {
            *inner.name.write() = to.to_owned();
            self.inner.collections.insert(to.to_owned(), inner);
        }
        Ok(())
    }

    /// Names of all collections, sorted for deterministic output.
    pub fn list_collections(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .inner
            .collections
            .iter()
            .map(|e| e.key().clone())
            .collect();
        names.sort();
        names
    }
}

/// Checkpoint on the last handle drop (WAL-050): seal state, truncate the WAL,
/// and leave a clean header. Errors are swallowed but leave a recoverable WAL
/// (WAL-051). A memory database does nothing. Only the last `VecLite` handle
/// triggers it — collection handles hold the persistence, not the database, and
/// the checkpoint trigger holds only a weak ref, so neither inflates the count.
#[cfg(not(target_arch = "wasm32"))]
impl Drop for VecLite {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            if let Some(p) = &self.inner.persistence {
                if !p.is_crashed() {
                    let _ = checkpoint_inner(&self.inner);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::Metric;
    use crate::point::Point;

    fn opts() -> CollectionOptions {
        CollectionOptions::new(2, Metric::Euclidean)
    }

    #[test]
    fn create_get_list_delete() {
        let db = VecLite::memory();
        db.create_collection("b", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_collection("a", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(db.list_collections(), vec!["a", "b"]);
        assert!(db.collection("a").is_ok());
        db.delete_collection("a").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(db.list_collections(), vec!["b"]);
        assert!(matches!(
            db.collection("a"),
            Err(VecLiteError::CollectionNotFound(_))
        ));
    }

    #[test]
    fn duplicate_names_rejected() {
        let db = VecLite::memory();
        db.create_collection("docs", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            db.create_collection("docs", opts()),
            Err(VecLiteError::AlreadyExists(_))
        ));
    }

    #[test]
    fn dimension_bounds_enforced_at_create() {
        let db = VecLite::memory();
        assert!(matches!(
            db.create_collection("zero", CollectionOptions::new(0, Metric::Euclidean)),
            Err(VecLiteError::InvalidArgument(_))
        ));
        assert!(matches!(
            db.create_collection("huge", CollectionOptions::new(65_537, Metric::Euclidean)),
            Err(VecLiteError::InvalidArgument(_))
        ));
        assert!(
            db.create_collection("max", CollectionOptions::new(65_536, Metric::Euclidean))
                .is_ok()
        );
    }

    #[test]
    fn invalid_names_rejected_at_create() {
        let db = VecLite::memory();
        assert!(matches!(
            db.create_collection("a/b", opts()),
            Err(VecLiteError::InvalidArgument(_))
        ));
    }

    #[test]
    fn stale_handle_errors_after_delete() {
        let db = VecLite::memory();
        let c = db
            .create_collection("docs", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("a", vec![1.0, 2.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.delete_collection("docs")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            c.upsert(Point::new("b", vec![1.0, 2.0])),
            Err(VecLiteError::CollectionNotFound(_))
        ));
        assert!(matches!(
            c.get("a"),
            Err(VecLiteError::CollectionNotFound(_))
        ));
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn rename_is_metadata_only_and_handles_survive() {
        let db = VecLite::memory();
        let c = db
            .create_collection("old", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("a", vec![1.0, 2.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.rename_collection("old", "new")
            .unwrap_or_else(|e| panic!("{e}"));

        assert!(matches!(
            db.collection("old"),
            Err(VecLiteError::CollectionNotFound(_))
        ));
        assert_eq!(c.name(), "new");
        assert_eq!(c.len(), 1);
        assert_eq!(
            db.collection("new").unwrap_or_else(|e| panic!("{e}")).len(),
            1
        );

        // Renaming onto an existing name fails.
        db.create_collection("other", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            db.rename_collection("new", "other"),
            Err(VecLiteError::AlreadyExists(_))
        ));
        // Renaming a missing collection fails.
        assert!(matches!(
            db.rename_collection("ghost", "x"),
            Err(VecLiteError::CollectionNotFound(_))
        ));
    }

    #[test]
    fn databases_are_independent() {
        let db1 = VecLite::memory();
        let db2 = VecLite::memory();
        db1.create_collection("docs", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(db2.collection("docs").is_err());
    }
}
