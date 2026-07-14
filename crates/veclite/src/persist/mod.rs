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
