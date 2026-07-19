//! Configuration types: collection options, open options, and their
//! server-parity defaults (SPEC-004 §1/§3).
//!
//! Every default in this module is pinned by the defaults table in
//! SPEC-004 §3 — changing one is a breaking change (API-010).

use std::path::PathBuf;

/// Default auto-embedding provider (server parity — SPEC-004 §3).
pub const DEFAULT_EMBEDDING_PROVIDER: &str = "bm25";

/// Default [`OpenOptions::memory_budget`]: 4 GiB of mapped vector bytes may be
/// indexed in RAM; past that a collection serves exact scans from the mmap
/// (SPEC-002 STG-064, ADR-0004).
pub const DEFAULT_MEMORY_BUDGET: u64 = 4 * 1024 * 1024 * 1024;

/// Distance metric for a collection (SPEC-001 CORE-015).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Metric {
    /// Cosine similarity; vectors are normalized at ingest (CORE-014).
    #[default]
    Cosine,
    /// Euclidean (L2) distance; results ordered ascending (CORE-035).
    Euclidean,
    /// Dot-product similarity; results ordered descending.
    DotProduct,
}

/// Vector quantization applied at ingest (SPEC-001 §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantization {
    /// No quantization: raw `f32` storage.
    None,
    /// Scalar quantization to `bits` ∈ {8, 4, 2, 1} per component.
    /// The default is `bits: 8` (server parity, CORE-040).
    Scalar {
        /// Bits per component: 8, 4, 2, or 1.
        bits: u8,
    },
    /// 1-bit binary codes (`dimension / 8` bytes per vector).
    Binary,
}

impl Default for Quantization {
    fn default() -> Self {
        Quantization::Scalar { bits: 8 }
    }
}

/// Block compression for compressible segments (SPEC-002 STG-020).
/// `VECTORS` segments are never compressed (STG-031).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// No compression.
    None,
    /// LZ4 for bodies at or above `threshold` bytes. Default, threshold 1024.
    Lz4 {
        /// Minimum body size in bytes before compression applies.
        threshold: u32,
    },
    /// Zstd for bodies at or above `threshold` bytes.
    Zstd {
        /// Minimum body size in bytes before compression applies.
        threshold: u32,
    },
}

impl Default for Compression {
    fn default() -> Self {
        Compression::Lz4 { threshold: 1024 }
    }
}

/// Durability mode for the write-ahead log (SPEC-003 §4).
///
/// Tunes freshness only — file integrity is guaranteed in every mode
/// (WAL-020).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Durability {
    /// fsync on every WAL append: every acked write survives an OS crash.
    Full,
    /// fsync at checkpoint and close (default): writes since the last
    /// checkpoint may be lost on OS crash; the file is never corrupt.
    #[default]
    Normal,
    /// No fsync: any un-checkpointed data may be lost; never corrupt.
    Off,
}

/// Kind of a declared payload index (SPEC-006 FLT-020).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadIndexKind {
    /// Exact-match index over string values.
    Keyword,
    /// Ordered index over integer values (supports ranges).
    Integer,
    /// Ordered index over float values (supports ranges).
    Float,
}

/// HNSW index parameters (SPEC-001 CORE-031). `m` and `ef_construction`
/// are fixed per collection; `ef_search` is a default overridable per query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HnswOptions {
    /// Graph connectivity. Default 16.
    pub m: usize,
    /// Candidate-list size during construction. Default 200.
    pub ef_construction: usize,
    /// Default candidate-list size during search. Default 100.
    pub ef_search: usize,
}

impl Default for HnswOptions {
    fn default() -> Self {
        HnswOptions {
            m: 16,
            ef_construction: 200,
            ef_search: 100,
        }
    }
}

/// Configuration for creating a collection (SPEC-004 §3).
///
/// ```
/// use veclite::{CollectionOptions, Metric, Quantization};
///
/// let opts = CollectionOptions::new(768, Metric::DotProduct)
///     .hnsw(16, 200, 100)
///     .quantization(Quantization::Scalar { bits: 8 });
/// ```
#[derive(Clone, Debug)]
// Fields are consumed by the collection registry from phase1a on; until that
// lands, only the builder writes them (allow scoped to this struct).
#[allow(dead_code)]
pub struct CollectionOptions {
    pub(crate) dimension: usize,
    pub(crate) metric: Metric,
    pub(crate) hnsw: HnswOptions,
    pub(crate) quantization: Quantization,
    pub(crate) compression: Compression,
    pub(crate) embedding_provider: Option<String>,
    pub(crate) payload_indexes: Vec<(String, PayloadIndexKind)>,
}

impl CollectionOptions {
    /// Bring-your-own-vectors collection with the given dimension and metric.
    pub fn new(dimension: usize, metric: Metric) -> Self {
        CollectionOptions {
            dimension,
            metric,
            hnsw: HnswOptions::default(),
            quantization: Quantization::default(),
            compression: Compression::default(),
            embedding_provider: None,
            payload_indexes: Vec::new(),
        }
    }

    /// Auto-embedding collection: text in, vectors managed internally
    /// (SPEC-005 §4). `provider` is e.g. [`DEFAULT_EMBEDDING_PROVIDER`].
    pub fn auto_embed(provider: &str, dimension: usize) -> Self {
        let mut opts = CollectionOptions::new(dimension, Metric::default());
        opts.embedding_provider = Some(provider.to_owned());
        opts
    }

    /// Override the distance metric.
    ///
    /// Needed alongside [`CollectionOptions::auto_embed`], which picks
    /// [`Metric::default`] because it takes no metric argument: without this,
    /// an auto-embed collection could not be given a non-default metric at all,
    /// and every binding silently dropped the caller's choice.
    #[must_use]
    pub fn metric(mut self, metric: Metric) -> Self {
        self.metric = metric;
        self
    }

    /// Tune the HNSW parameters (`m`, `ef_construction`, default `ef_search`).
    pub fn hnsw(mut self, m: usize, ef_construction: usize, ef_search: usize) -> Self {
        self.hnsw = HnswOptions {
            m,
            ef_construction,
            ef_search,
        };
        self
    }

    /// Select the quantization applied at ingest. Default SQ-8.
    pub fn quantization(mut self, q: Quantization) -> Self {
        self.quantization = q;
        self
    }

    /// Declare a payload index built from creation time (repeatable).
    pub fn payload_index(mut self, key: &str, kind: PayloadIndexKind) -> Self {
        self.payload_indexes.push((key.to_owned(), kind));
        self
    }
}

/// Tuned database open (SPEC-004 §1). Zero-config `VecLite::open(path)`
/// is equivalent to `OpenOptions::new().open(path)` once storage lands.
#[derive(Clone, Debug)]
// Fields are consumed by the storage layer from phase2 on; until that lands,
// only the builder writes them (allow scoped to this struct).
#[allow(dead_code)]
pub struct OpenOptions {
    pub(crate) read_only: bool,
    /// `None` = auto: mmap files larger than 64 MiB (SPEC-004 §1).
    pub(crate) mmap: Option<bool>,
    /// In-RAM index budget for mmap'd collections (SPEC-002 STG-064,
    /// ADR-0004): a collection whose mmap'd vectors exceed this many bytes
    /// skips the HNSW build and serves exact brute-force k-NN from the map.
    pub(crate) memory_budget: u64,
    pub(crate) durability: Durability,
    pub(crate) background_checkpoint: bool,
    pub(crate) wal_size_limit: u64,
    pub(crate) auto_vacuum_threshold: f32,
    pub(crate) read_only_ignore_wal: bool,
    pub(crate) model_cache_dir: Option<PathBuf>,
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions {
            read_only: false,
            mmap: None,
            memory_budget: DEFAULT_MEMORY_BUDGET,
            durability: Durability::default(),
            background_checkpoint: false,
            wal_size_limit: 64 * 1024 * 1024,
            auto_vacuum_threshold: 0.25,
            read_only_ignore_wal: false,
            model_cache_dir: None,
        }
    }
}

impl OpenOptions {
    /// Start from the zero-config defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open with a shared lock, refuse writes (SPEC-002 STG-062).
    pub fn read_only(mut self, v: bool) -> Self {
        self.read_only = v;
        self
    }

    /// Force mmap reads on or off. Default: auto (files > 64 MiB).
    pub fn mmap(mut self, v: bool) -> Self {
        self.mmap = Some(v);
        self
    }

    /// In-RAM index budget for mmap'd collections (SPEC-002 STG-064): when a
    /// collection's mapped vectors exceed this many bytes, open skips the HNSW
    /// build and serves **exact** brute-force k-NN straight from the map —
    /// this is the larger-than-RAM tier (ADR-0004). Below the budget the graph
    /// is rebuilt in RAM and ANN search is served as usual. Default: 4 GiB.
    pub fn memory_budget(mut self, bytes: u64) -> Self {
        self.memory_budget = bytes;
        self
    }

    /// Select the WAL durability mode. Default [`Durability::Normal`].
    pub fn durability(mut self, v: Durability) -> Self {
        self.durability = v;
        self
    }

    /// Opt into an opportunistic background checkpoint thread. Default off:
    /// VecLite spawns no threads unless asked (NFR-07).
    pub fn background_checkpoint(mut self, v: bool) -> Self {
        self.background_checkpoint = v;
        self
    }

    /// WAL size that triggers a checkpoint. Default 64 MiB (SPEC-003 WAL-030).
    pub fn wal_size_limit(mut self, bytes: u64) -> Self {
        self.wal_size_limit = bytes;
        self
    }

    /// Tombstone ratio that escalates a checkpoint to a vacuum.
    /// Default 0.25 (SPEC-002 STG-072).
    pub fn auto_vacuum_threshold(mut self, ratio: f32) -> Self {
        self.auto_vacuum_threshold = ratio;
        self
    }

    /// Allow read-only opens to ignore a pending WAL, reading the last
    /// checkpoint state (SPEC-003 WAL-043). Default false.
    pub fn read_only_ignore_wal(mut self, v: bool) -> Self {
        self.read_only_ignore_wal = v;
        self
    }

    /// Cache directory for downloaded embedding models (`onnx` feature).
    pub fn model_cache_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.model_cache_dir = Some(path.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SPEC-004 §3 defaults table, pinned (task 2.1).
    #[test]
    fn collection_defaults_match_server_parity_table() {
        let opts = CollectionOptions::new(384, Metric::default());
        assert_eq!(opts.dimension, 384);
        assert_eq!(opts.metric, Metric::Cosine);
        assert_eq!(opts.hnsw.m, 16);
        assert_eq!(opts.hnsw.ef_construction, 200);
        assert_eq!(opts.hnsw.ef_search, 100);
        assert_eq!(opts.quantization, Quantization::Scalar { bits: 8 });
        assert_eq!(opts.compression, Compression::Lz4 { threshold: 1024 });
        assert_eq!(opts.embedding_provider, None);
        assert!(opts.payload_indexes.is_empty());
    }

    #[test]
    fn auto_embed_records_provider_and_cosine_default() {
        let opts = CollectionOptions::auto_embed(DEFAULT_EMBEDDING_PROVIDER, 512);
        assert_eq!(opts.embedding_provider.as_deref(), Some("bm25"));
        assert_eq!(opts.dimension, 512);
        assert_eq!(opts.metric, Metric::Cosine);
    }

    #[test]
    fn collection_builder_applies_tuning() {
        let opts = CollectionOptions::new(768, Metric::DotProduct)
            .hnsw(32, 400, 200)
            .quantization(Quantization::None)
            .payload_index("lang", PayloadIndexKind::Keyword)
            .payload_index("year", PayloadIndexKind::Integer);
        assert_eq!(opts.metric, Metric::DotProduct);
        assert_eq!(opts.hnsw.m, 32);
        assert_eq!(opts.hnsw.ef_construction, 400);
        assert_eq!(opts.hnsw.ef_search, 200);
        assert_eq!(opts.quantization, Quantization::None);
        assert_eq!(
            opts.payload_indexes,
            vec![
                ("lang".to_owned(), PayloadIndexKind::Keyword),
                ("year".to_owned(), PayloadIndexKind::Integer),
            ]
        );
    }

    /// The SPEC-004 §1 open defaults, pinned (task 2.1).
    #[test]
    fn open_defaults_match_spec() {
        let opts = OpenOptions::new();
        assert!(!opts.read_only);
        assert_eq!(opts.mmap, None); // auto
        assert_eq!(opts.durability, Durability::Normal);
        assert!(!opts.background_checkpoint);
        assert_eq!(opts.wal_size_limit, 64 * 1024 * 1024);
        assert!((opts.auto_vacuum_threshold - 0.25).abs() < f32::EPSILON);
        assert!(!opts.read_only_ignore_wal);
        assert_eq!(opts.model_cache_dir, None);
    }

    #[test]
    fn open_builder_applies_tuning() {
        let opts = OpenOptions::new()
            .read_only(true)
            .mmap(false)
            .durability(Durability::Full)
            .background_checkpoint(true)
            .wal_size_limit(1024)
            .auto_vacuum_threshold(0.5)
            .read_only_ignore_wal(true)
            .model_cache_dir("/tmp/models");
        assert!(opts.read_only);
        assert_eq!(opts.mmap, Some(false));
        assert_eq!(opts.durability, Durability::Full);
        assert!(opts.background_checkpoint);
        assert_eq!(opts.wal_size_limit, 1024);
        assert!((opts.auto_vacuum_threshold - 0.5).abs() < f32::EPSILON);
        assert!(opts.read_only_ignore_wal);
        assert_eq!(opts.model_cache_dir, Some(PathBuf::from("/tmp/models")));
    }
}
