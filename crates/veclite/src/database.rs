//! Database handle and collection registry (SPEC-001 §4, SPEC-004 §1–2).
//!
//! Concurrency model (CORE-051): collection lookups are lock-free reads on a
//! `DashMap`; the rare registry mutations (create/delete/rename) serialize on
//! one mutex so two-key operations like rename stay atomic.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dashmap::DashMap;
use parking_lot::Mutex;

use crate::collection::{Collection, CollectionInner, EmbedderSlot, SharedEmbedder, WalSink};
use crate::embedding::Embedder;
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
use crate::storage::image::CheckpointColl;
#[cfg(not(target_arch = "wasm32"))]
use crate::storage::wal::{WalEntry, WalOp};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// Maximum collection dimension (SPEC-002 §8 limits).
const MAX_DIMENSION: usize = 65_536;

struct DatabaseInner {
    collections: DashMap<String, Arc<CollectionInner>>,
    /// Alias name → target collection name (SPEC-004 §2, CORE-011). Resolved
    /// transparently in `collection()`.
    aliases: DashMap<String, String>,
    /// Custom embedding providers registered on THIS database instance
    /// (SPEC-005 EMB-011 — never global). One shared instance per name;
    /// collections referencing it hold clones of the `Arc`.
    registered_embedders: DashMap<String, SharedEmbedder>,
    /// Serializes create/delete/rename (registry-level write, CORE-051).
    registry: Mutex<()>,
    /// Next collection id to assign (stamped into WAL entries + CONFIG).
    next_coll_id: AtomicU32,
    /// Where `fastembed:<model>` ONNX weights download (SPEC-005 EMB-041); from
    /// `OpenOptions::model_cache_dir`, `None` for `memory()` or when unset (the
    /// fastembed default cache). Threaded to onnx provider construction.
    model_cache_dir: Option<std::path::PathBuf>,
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
    aliases: Vec<String>,
    loaded: seal::LoadedCollection,
) -> Result<()> {
    let sink: Arc<dyn WalSink> = Arc::clone(persistence) as Arc<dyn WalSink>;
    let capacity = loaded
        .base
        .as_ref()
        .map_or(loaded.points.len(), |b| b.slot_count);
    // Resolve the provider LENIENTLY: reopen must never fail on an
    // unavailable provider (EMB-011/023) — built-in, then registered, then a
    // deferred `Missing` slot whose text operations fail with the remedy.
    let slot = match &loaded.options.embedding_provider {
        None => EmbedderSlot::None,
        Some(provider) => {
            match crate::embedding::build_provider_with(
                provider,
                loaded.options.dimension,
                inner.model_cache_dir.as_deref(),
            ) {
                Ok(built) => EmbedderSlot::Ready(Arc::new(Mutex::new(built))),
                Err(_) => match inner.registered_embedders.get(provider) {
                    Some(shared) => EmbedderSlot::Ready(Arc::clone(shared.value())),
                    None => EmbedderSlot::Missing(provider.clone()),
                },
            }
        }
    };
    let auto_embed = loaded.options.embedding_provider.is_some();
    let cinner = Arc::new(CollectionInner::with_embedder(
        name.clone(),
        loaded.options,
        coll_id,
        Some(sink),
        capacity,
        slot,
    )?);
    // Restore the collection's aliases and register them for resolution.
    *cinner.aliases.write() = aliases.clone();
    for alias in aliases {
        inner.aliases.insert(alias, name.clone());
    }
    let handle = Collection {
        inner: Arc::clone(&cinner),
    };
    let has_points = loaded.base.is_some() || !loaded.points.is_empty();
    // Import the checkpointed VOCAB before replaying points, mirroring the
    // live order (state at checkpoint, then incremental updates per doc). A
    // legacy file (text docs, no VOCAB segment) refits on its first search.
    match &loaded.vocab {
        Some(state) => handle.replay_import_vocab(state)?,
        None if auto_embed && has_points => handle.mark_text_dirty(),
        None => {}
    }
    if let Some(base) = loaded.base {
        // mmap tier (ADR-0004): vectors stay in the file map.
        handle.install_base(base)?;
    } else {
        let points: Vec<Point> = loaded
            .points
            .into_iter()
            .map(|(id, vector, payload, sparse)| Point {
                id,
                vector,
                sparse,
                payload,
            })
            .collect();
        // Checkpoint-loaded points: the sealed VOCAB already accounts for
        // them, so install WITHOUT vocabulary updates (WAL replay is the path
        // that folds documents in incrementally).
        handle.install_points(points)?;
    }
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
                body.aliases,
                seal::LoadedCollection {
                    options,
                    points: Vec::new(),
                    base: None,
                    vocab: None,
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
        WalOp::Alias => {
            let body: wal_body::Alias = wal_body::decode(&entry.body, "alias")?;
            let target = inner
                .collections
                .iter()
                .find(|e| e.value().coll_id == entry.coll_id)
                .map(|e| (e.key().clone(), Arc::clone(e.value())));
            if let Some((name, ci)) = target {
                if body.create {
                    inner.aliases.insert(body.alias.clone(), name);
                    ci.aliases.write().push(body.alias);
                } else {
                    inner.aliases.remove(&body.alias);
                    ci.aliases.write().retain(|a| a != &body.alias);
                }
            }
        }
        WalOp::PidxDeclare => {
            let body: wal_body::PidxDeclare = wal_body::decode(&entry.body, "pidx")?;
            if let Some(c) = collection_by_id(inner, entry.coll_id) {
                c.replay_declare_index(&body.key, config::pidx_kind_from(body.kind)?);
            }
        }
        WalOp::VocabUpdate => {
            // A full embedder-state snapshot (SPEC-005 EMB-032, journaled by
            // `refit` after its re-upsert batches): importing it overwrites
            // the incrementally replayed state with the exact fitted one.
            if let Some(c) = collection_by_id(inner, entry.coll_id) {
                c.replay_import_vocab(&entry.body)?;
            }
        }
    }
    Ok(())
}

/// Seal every live collection's current state into `CheckpointColl`s — the
/// input shared by checkpoint, vacuum, and snapshot. `allow_reuse` lets an
/// unmutated mmap-tier collection be carried forward by segment reference
/// (ADR-0004) — valid only when committing into the **same** file, so
/// checkpoint passes `true` while snapshot and vacuum (fresh files, all
/// offsets invalidated) pass `false`.
#[cfg(not(target_arch = "wasm32"))]
fn sealed_live_collections(
    inner: &Arc<DatabaseInner>,
    allow_reuse: bool,
) -> Result<Vec<CheckpointColl>> {
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
        let name = ci.name.read().clone();
        let aliases = ci.aliases.read().clone();
        if allow_reuse {
            if let Some((refs, vector_count, tombstone_count)) = handle.clean_reuse() {
                colls.push(CheckpointColl {
                    coll_id: ci.coll_id,
                    name,
                    aliases,
                    vector_count,
                    tombstone_count,
                    segments: Vec::new(),
                    reused: Some(refs),
                });
                continue;
            }
        }
        let live = handle.live_points();
        let vocab = handle.export_vocab_state()?;
        colls.push(seal::seal(
            ci.coll_id,
            name,
            aliases,
            &ci.config,
            &handle.declared_indexes(),
            vocab.as_deref(),
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
    persistence.commit(sealed_live_collections(inner, true)?)
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
    persistence.vacuum(sealed_live_collections(inner, false)?)
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
                aliases: DashMap::new(),
                registered_embedders: DashMap::new(),
                registry: Mutex::new(()),
                next_coll_id: AtomicU32::new(1),
                model_cache_dir: None,
                #[cfg(not(target_arch = "wasm32"))]
                persistence: None,
            }),
        }
    }

    /// Serialize the whole database to a self-contained `.veclite` v1 file image
    /// (SPEC-002). The bytes are a compacted, single-generation image readable by
    /// native [`VecLite::open`] (write them to a file) and by
    /// [`VecLite::deserialize`] — the interchange contract (WASM-010). Every
    /// collection is sealed fresh (tombstones dropped); the HNSW graph is not
    /// stored (STG-063) — a reader rebuilds it. Portable: this is the wasm
    /// persistence path, but it runs identically on native.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        use crate::storage::compression::Codec;
        // On wasm there is no clock/RNG dependency (getrandom is not linked); a
        // zero uuid/epoch is a valid header (uuid is only a multi-file identity
        // hint, never read back into behavior). Native stamps real values.
        #[cfg(not(target_arch = "wasm32"))]
        let (epoch, file_uuid) = (now_epoch_s(), *uuid::Uuid::new_v4().as_bytes());
        #[cfg(target_arch = "wasm32")]
        let (epoch, file_uuid) = (0u64, [0u8; 16]);

        let mut colls = Vec::new();
        for entry in self.inner.collections.iter() {
            let ci = entry.value();
            if ci.deleted.load(Ordering::Acquire) {
                continue;
            }
            let handle = Collection {
                inner: Arc::clone(ci),
            };
            let name = ci.name.read().clone();
            let aliases = ci.aliases.read().clone();
            let live = handle.live_points();
            let vocab = handle.export_vocab_state()?;
            colls.push(crate::persist::seal::seal(
                ci.coll_id,
                name,
                aliases,
                &ci.config,
                // The declared payload indexes live in the immutable config; the
                // runtime accelerator (`declared_indexes`) is native-only, so read
                // the declarations directly to seal PIDX on every target.
                &ci.config.payload_indexes,
                vocab.as_deref(),
                &live,
                epoch,
            )?);
        }
        crate::storage::image::write_image(file_uuid, epoch, epoch, 1, colls, Codec::Lz4)
    }

    /// Load a database from a `.veclite` v1 file image produced by
    /// [`serialize`](Self::serialize) or by a native file's committed bytes
    /// (WASM-010). The result is an in-memory database (no file binding); call
    /// [`serialize`](Self::serialize) again to persist. Collections, aliases,
    /// payloads, sparse lanes, and auto-embed vocabulary are all restored; the
    /// HNSW graph (native) is rebuilt from the vectors as they install.
    pub fn deserialize(bytes: &[u8]) -> Result<Self> {
        let db = VecLite::memory();
        for ic in crate::storage::image::read_image(bytes)? {
            let loaded = crate::persist::seal::load(&ic.segments)?;
            let handle = db.create_collection(&ic.entry.name, loaded.options)?;
            for alias in &ic.entry.aliases {
                db.create_alias(alias, &ic.entry.name)?;
            }
            // Import the checkpointed embedder state before points so text search
            // needs no rebuild (mirrors the native load order, EMB-030).
            if let Some(state) = &loaded.vocab {
                handle.replay_import_vocab(state)?;
            }
            if !loaded.points.is_empty() {
                let points: Vec<crate::point::Point> = loaded
                    .points
                    .into_iter()
                    .map(|(id, vector, payload, sparse)| crate::point::Point {
                        id,
                        vector,
                        sparse,
                        payload,
                    })
                    .collect();
                // Install without vocabulary updates — the sealed VOCAB already
                // accounts for these points (double-counting guard, EMB-030).
                handle.install_points(points)?;
            }
        }
        Ok(db)
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
            &crate::persist::OpenConfig {
                durability: options.durability,
                wal_size_limit: options.wal_size_limit,
                auto_vacuum_threshold: options.auto_vacuum_threshold,
                read_only: options.read_only,
                read_only_ignore_wal: options.read_only_ignore_wal,
                mmap: options.mmap,
                memory_budget: options.memory_budget,
            },
        )?;
        let persistence = Arc::new(persistence);
        let inner = Arc::new(DatabaseInner {
            collections: DashMap::new(),
            aliases: DashMap::new(),
            registered_embedders: DashMap::new(),
            registry: Mutex::new(()),
            next_coll_id: AtomicU32::new(1),
            model_cache_dir: options.model_cache_dir.clone(),
            persistence: Some(Arc::clone(&persistence)),
        });

        let mut max_coll_id = 0u32;
        for (name, coll_id, aliases, loaded) in state.collections {
            install_collection(&inner, &persistence, name, coll_id, aliases, loaded)?;
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
        Persistence::write_snapshot(path.as_ref(), sealed_live_collections(&self.inner, false)?)
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
        // Resolve the auto-embed provider before journaling (EMB-021), so a
        // bad `CreateColl` never enters the WAL: built-in first, then this
        // database's registered providers (EMB-011).
        let slot = match &options.embedding_provider {
            None => EmbedderSlot::None,
            Some(provider) => match crate::embedding::build_provider_with(
                provider,
                options.dimension,
                self.inner.model_cache_dir.as_deref(),
            ) {
                Ok(built) => EmbedderSlot::Ready(Arc::new(Mutex::new(built))),
                Err(e) => match self.inner.registered_embedders.get(provider) {
                    Some(shared) => EmbedderSlot::Ready(Arc::clone(shared.value())),
                    None => return Err(self.with_registered_in_available(e)),
                },
            },
        };
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
            // The CREATE_COLL config projection carries no index declarations
            // (StoredConfig is frozen); journal each creation-time declaration
            // as its own PIDX_DECLARE so crash-replay restores them (FLT-020).
            for (key, kind) in &options.payload_indexes {
                let body = wal_body::encode(&wal_body::PidxDeclare {
                    key: key.clone(),
                    kind: config::pidx_kind_byte(*kind),
                })?;
                p.append(coll_id, WalOp::PidxDeclare, body)?;
            }
        }
        let inner = Arc::new(CollectionInner::with_embedder(
            name.to_owned(),
            options,
            coll_id,
            self.inner.sink(),
            0,
            slot,
        )?);
        self.inner
            .collections
            .insert(name.to_owned(), Arc::clone(&inner));
        Ok(Collection { inner })
    }

    /// Extend an `UnsupportedProvider` error's `available` list with this
    /// database's registered provider names.
    fn with_registered_in_available(&self, e: VecLiteError) -> VecLiteError {
        match e {
            VecLiteError::UnsupportedProvider {
                requested,
                mut available,
            } => {
                for entry in self.inner.registered_embedders.iter() {
                    available.push(entry.key().clone());
                }
                available.sort();
                VecLiteError::UnsupportedProvider {
                    requested,
                    available,
                }
            }
            other => other,
        }
    }

    /// Register a custom embedding provider on THIS database instance
    /// (SPEC-005 EMB-011 — never global). One shared instance serves every
    /// collection referencing `name`; collections loaded earlier with this
    /// provider deferred (`UnsupportedProvider` on text operations) bind to it
    /// now, importing any VOCAB state carried through open. Registering a
    /// built-in or already-registered name is `AlreadyExists`.
    pub fn register_embedder(&self, name: &str, embedder: Box<dyn Embedder>) -> Result<()> {
        if crate::embedding::available_providers()
            .iter()
            .any(|p| p == name)
            || crate::embedding::is_onnx_provider(name)
        {
            return Err(VecLiteError::AlreadyExists(format!(
                "embedding provider {name:?} is built-in"
            )));
        }
        if self.inner.registered_embedders.contains_key(name) {
            return Err(VecLiteError::AlreadyExists(format!(
                "embedding provider {name:?} is already registered"
            )));
        }
        let shared: SharedEmbedder = Arc::new(Mutex::new(embedder));
        self.inner
            .registered_embedders
            .insert(name.to_owned(), Arc::clone(&shared));
        // Bind to collections that loaded with this provider deferred.
        for entry in self.inner.collections.iter() {
            Collection {
                inner: Arc::clone(entry.value()),
            }
            .bind_embedder(name, &shared)?;
        }
        Ok(())
    }

    /// Handle to a collection by name or alias (lock-free lookup, CORE-051). An
    /// alias resolves transparently to its target (SPEC-004 §2).
    pub fn collection(&self, name: &str) -> Result<Collection> {
        if let Some(entry) = self.inner.collections.get(name) {
            return Ok(Collection {
                inner: Arc::clone(entry.value()),
            });
        }
        if let Some(alias) = self.inner.aliases.get(name) {
            if let Some(entry) = self.inner.collections.get(alias.value()) {
                return Ok(Collection {
                    inner: Arc::clone(entry.value()),
                });
            }
        }
        Err(VecLiteError::CollectionNotFound(name.to_owned()))
    }

    /// Create an alias that resolves to `target` in `collection()` lookups
    /// (SPEC-004 §2, CORE-011). `AlreadyExists` if the name is taken by a
    /// collection or another alias; `CollectionNotFound` if `target` is missing.
    /// Re-point by deleting the alias first.
    pub fn create_alias(&self, alias: &str, target: &str) -> Result<()> {
        validate_collection_name(alias)?;
        let _guard = self.inner.registry.lock();
        if self.inner.collections.contains_key(alias) || self.inner.aliases.contains_key(alias) {
            return Err(VecLiteError::AlreadyExists(alias.to_owned()));
        }
        let coll_id = self
            .inner
            .collections
            .get(target)
            .map(|e| e.coll_id)
            .ok_or_else(|| VecLiteError::CollectionNotFound(target.to_owned()))?;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = &self.inner.persistence {
            let body = wal_body::encode(&wal_body::Alias {
                create: true,
                alias: alias.to_owned(),
            })?;
            p.append(coll_id, WalOp::Alias, body)?;
        }
        #[cfg(target_arch = "wasm32")]
        let _ = coll_id;
        self.inner
            .aliases
            .insert(alias.to_owned(), target.to_owned());
        if let Some(ci) = self.inner.collections.get(target) {
            ci.aliases.write().push(alias.to_owned());
        }
        Ok(())
    }

    /// Delete an alias (SPEC-004 §2). `CollectionNotFound` if it does not exist.
    pub fn delete_alias(&self, alias: &str) -> Result<()> {
        let _guard = self.inner.registry.lock();
        let target = self
            .inner
            .aliases
            .get(alias)
            .map(|e| e.value().clone())
            .ok_or_else(|| VecLiteError::CollectionNotFound(alias.to_owned()))?;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = &self.inner.persistence {
            if let Some(coll_id) = self.inner.collections.get(&target).map(|e| e.coll_id) {
                let body = wal_body::encode(&wal_body::Alias {
                    create: false,
                    alias: alias.to_owned(),
                })?;
                p.append(coll_id, WalOp::Alias, body)?;
            }
        }
        self.inner.aliases.remove(alias);
        if let Some(ci) = self.inner.collections.get(&target) {
            ci.aliases.write().retain(|a| a != alias);
        }
        Ok(())
    }

    /// All `(alias, target)` pairs, sorted by alias.
    pub fn aliases(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .inner
            .aliases
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();
        out.sort();
        out
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
            // Release a deleted collection's mmap base so a lingering handle
            // cannot pin the file mapping across a vacuum swap (STG-071).
            #[cfg(not(target_arch = "wasm32"))]
            Collection {
                inner: Arc::clone(&inner),
            }
            .drop_base_unchecked();
            // Drop any aliases that resolved to this collection.
            for alias in inner.aliases.read().iter() {
                self.inner.aliases.remove(alias);
            }
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
            // Re-point this collection's aliases at the new name.
            for alias in inner.aliases.read().iter() {
                self.inner.aliases.insert(alias.clone(), to.to_owned());
            }
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
