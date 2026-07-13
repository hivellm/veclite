//! VecLite — embedded, single-file, in-process vector database.
//!
//! VecLite is the in-process, single-file distribution of the
//! [Vectorizer](https://github.com/hivellm/vectorizer) engine: HNSW search,
//! quantization, hybrid dense+sparse retrieval, and payload filtering — as a
//! library you link, not a server you run.
//!
//! The normative contract for this crate lives in `docs/specs/` (PRD, DAG,
//! SPEC-001..016). This crate is under construction phase by phase; the
//! current build ships the in-memory engine: the collection registry and
//! vector CRUD over an ephemeral [`VecLite::memory`] database (phase1a).

pub mod collection;
pub mod database;
pub mod error;
pub mod options;
pub mod point;

pub use collection::Collection;
pub use database::VecLite;
pub use error::{Result, VecLiteError};
pub use options::{
    CollectionOptions, Compression, DEFAULT_EMBEDDING_PROVIDER, Durability, HnswOptions, Metric,
    OpenOptions, PayloadIndexKind, Quantization,
};
pub use point::{Hit, Point, SparseVector};
