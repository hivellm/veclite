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
pub mod bow;
pub mod char_ngram;
#[cfg(feature = "svd")]
pub mod svd;
pub mod tfidf;

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

    /// Fold one more document into the trainable state **incrementally**
    /// (SPEC-005 EMB-030): update document-frequency tables and append new
    /// terms while vocabulary space remains — approximate by design (existing
    /// term indices never move, so stored vectors stay comparable); `fit` /
    /// `refit` remains the exact recompute (EMB-031). Default: no-op, for
    /// stateless providers.
    fn add_document(&mut self, text: &str) {
        let _ = text;
    }

    /// Serialize provider-private state for the VOCAB segment (SPEC-005 EMB-010);
    /// empty for stateless providers. MUST be forward/backward compatible.
    fn export_state(&self) -> Result<Vec<u8>>;

    /// Restore state produced by [`export_state`](Embedder::export_state).
    fn import_state(&mut self, state: &[u8]) -> Result<()>;
}

/// The default auto-embed provider (SPEC-005 §2).
pub const DEFAULT_PROVIDER: &str = "bm25";

/// The built-in sparse provider ids available in the default build (drives the
/// `UnsupportedProvider` error's `available` list — SPEC-005 EMB-021). `svd`
/// (feature-gated) and `fastembed:*` (onnx) are added by their features.
#[must_use]
pub fn available_providers() -> Vec<String> {
    #[allow(unused_mut)] // mutated only when the `svd` feature is on
    let mut out: Vec<String> = ["bm25", "tfidf", "bow", "char_ngram"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    #[cfg(feature = "svd")]
    out.push("svd".to_owned());
    out
}

/// Whether `provider` names the ONNX/fastembed family (SPEC-005 §2), which is
/// only available behind the `onnx` feature (EMB-040) — used by the reopen
/// path to defer rather than fail the whole open (EMB-023).
#[must_use]
pub fn is_onnx_provider(provider: &str) -> bool {
    provider.starts_with("fastembed:")
}

/// Construct a built-in provider by id, or fail with `UnsupportedProvider`
/// listing what is available — never a silent fallback (SPEC-005 EMB-021).
pub fn build_provider(provider: &str, dimension: usize) -> Result<Box<dyn Embedder>> {
    match provider {
        "bm25" => Ok(Box::new(bm25::Bm25::new(dimension))),
        "tfidf" => Ok(Box::new(tfidf::TfIdf::new(dimension))),
        "bow" => Ok(Box::new(bow::BagOfWords::new(dimension))),
        "char_ngram" => Ok(Box::new(char_ngram::CharNgram::new(
            dimension,
            char_ngram::DEFAULT_N,
        ))),
        #[cfg(feature = "svd")]
        "svd" => Ok(Box::new(svd::Svd::new(dimension))),
        other => Err(VecLiteError::UnsupportedProvider {
            requested: other.to_owned(),
            available: available_providers(),
        }),
    }
}
