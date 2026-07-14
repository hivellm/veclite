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
// HNSW index over the pinned hnsw_rs. Native-only: hnsw_rs cannot build on
// wasm32 (ADR-0002). Consumed by the collection index integration.
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
pub(crate) mod index;
pub mod options;
pub mod point;
pub mod query;
// Vendored quantization encodings (SQ-8 default, scalar 4/2/1-bit, binary;
// product behind the `pq` feature). Consumed by the collection's reindex-time
// encoding; byte-identical to the server (CORE-041). Cosmetic clippy lints are
// allowed here to keep the files diffable against upstream for the manual
// port-back required by CORE-003 — the encoding math is untouched.
#[allow(
    dead_code,
    unused_imports,
    unused_parens,
    unused_variables,
    clippy::manual_div_ceil,
    clippy::needless_range_loop,
    clippy::items_after_test_module,
    clippy::ptr_arg
)]
pub(crate) mod quantization;
// Vendored scalar distance/quantization kernels. The quantization module
// (this phase) consumes the quantize/dequantize entry points; the distance
// primitives are consumed by the recall harness and phase1c search, and the
// full trait is kept complete for the future ISA backends that override it.
#[allow(dead_code)]
pub(crate) mod simd;
// On-disk `.veclite` format v1 (SPEC-002). Native-only: wasm32 has no file
// storage (CORE-004), and zstd links a C library. Consumed by the persistent
// database open/checkpoint path (phase2b+); the codec + commit protocol land
// here first.
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
pub(crate) mod storage;
// Retrieval-quality gates (recall, SQ-8 recall, tombstone). Native-only:
// they drive the HNSW index via the crate-internal `search_internal`.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod recall;

pub use collection::Collection;
pub use database::VecLite;
pub use error::{Result, VecLiteError};
pub use options::{
    CollectionOptions, Compression, DEFAULT_EMBEDDING_PROVIDER, Durability, HnswOptions, Metric,
    OpenOptions, PayloadIndexKind, Quantization,
};
pub use point::{Hit, Point, SparseVector};
pub use query::{Filter, QueryBuilder};
