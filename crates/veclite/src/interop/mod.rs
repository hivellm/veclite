//! Vectorizer `.vecdb` interop (SPEC-013) and the file-integrity pass behind
//! the `veclite` CLI (SPEC-014).
//!
//! The graduation path: [`export_vecdb`] writes a server data directory
//! (`vectorizer.vecdb` ZIP + `vectorizer.vecidx` sidecar, Compact layout) the
//! server's `StorageReader` accepts; [`import_vecdb`] reads both server
//! layouts (Compact and Legacy) back into a VecLite database, degrading
//! server-only aspects with warnings — never silently (IOP-022).

mod config;
mod export;
mod import;
mod model;
mod vocab;

pub use export::{ExportOptions, ExportReport, ExportedCollection, export_vecdb};
pub use import::{
    ImportOptions, ImportReport, ImportedCollection, ServerLayout, detect_layout, import_vecdb,
};
