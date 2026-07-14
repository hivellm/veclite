//! Text-embedding providers (SPEC-005). Pure-Rust sparse/lexical embedders that
//! turn text into vectors offline, with no network and no external model files.
//! The provider names and scoring match the Vectorizer server so a collection's
//! `embedding_provider` string survives the graduation path (SPEC-013).
//!
//! This module ships the [`Embedder`] trait (SPEC-005 §3) and the built-in
//! provider factory. `bm25` (the default) is vendored from the server here;
//! `tfidf`/`bow`/`char_ngram`/`svd` and the auto-embed collection API
//! (`upsert_text`/`search_text`), the vocabulary WAL lifecycle, and the chunker
//! land in follow-up increments (see `phase3b`).

pub mod bm25;

use crate::error::{Result, VecLiteError};

/// A text embedder (SPEC-005 §3). Synchronous and object-safe so a collection
/// can hold a `Box<dyn Embedder>`. Trainable providers accumulate vocabulary
/// state via [`fit`](Embedder::fit); stateless ones treat it as a no-op.
pub trait Embedder: Send + Sync {
    /// Embed one text into a `dimension()`-length vector.
    fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed a batch of texts (default: one call per text).
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// Output vector dimension.
    fn dimension(&self) -> usize;

    /// (Re)build trainable state from the **full** corpus (SPEC-005 EMB-031, the
    /// exact recompute `refit` uses). No-op for stateless providers.
    fn fit(&mut self, corpus: &[&str]) -> Result<()>;

    /// Serialize provider-private state for the VOCAB segment (SPEC-005 EMB-010);
    /// empty for stateless providers. MUST be forward/backward compatible.
    fn export_state(&self) -> Result<Vec<u8>>;

    /// Restore state produced by [`export_state`](Embedder::export_state).
    fn import_state(&mut self, state: &[u8]) -> Result<()>;
}

/// Built-in provider ids available in the default build. `tfidf`/`bow`/
/// `char_ngram` are reserved here and implemented in a follow-up increment; only
/// the ids in [`available_providers`] resolve today.
pub const DEFAULT_PROVIDER: &str = "bm25";

/// The provider ids this build can construct right now (drives the
/// `UnsupportedProvider` error's `available` list — SPEC-005 EMB-021).
#[must_use]
pub fn available_providers() -> Vec<String> {
    vec!["bm25".to_owned()]
}

/// Construct a built-in provider by id, or fail with `UnsupportedProvider`
/// listing what is available — never a silent fallback (SPEC-005 EMB-021).
pub fn build_provider(provider: &str, dimension: usize) -> Result<Box<dyn Embedder>> {
    match provider {
        "bm25" => Ok(Box::new(bm25::Bm25::new(dimension))),
        other => Err(VecLiteError::UnsupportedProvider {
            requested: other.to_owned(),
            available: available_providers(),
        }),
    }
}
