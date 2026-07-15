//! SVD embedding provider (SPEC-005 §2, feature `svd`) — TF-IDF plus a
//! truncated-SVD-style projection. Vendored from
//! `vectorizer/src/embedding/providers/svd.rs` (ADR-0001): the hash-seeded
//! pseudo-random orthonormal projection, the Gram–Schmidt pass, and the
//! matrix multiply are kept identical. Adaptations: the server's
//! `ndarray::Array2` container is replaced by a plain `Vec<Vec<f32>>` (same
//! math, zero extra dependencies), and an unfitted `embed` returns a zero
//! vector instead of an error so the provider follows the collection's
//! embed-then-refit lifecycle (SPEC-005 §5) like the other trainable
//! providers.

use serde::{Deserialize, Serialize};

use super::{Embedder, tfidf::TfIdf};
use crate::error::Result;

/// SVD provider state: a TF-IDF base transformation projected down to
/// `dimension` through a corpus-seeded orthonormal matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Svd {
    dimension: usize,
    tfidf: TfIdf,
    /// Row-major `dimension × vocab` projection (the server's truncated V^T).
    transformation: Option<Vec<Vec<f32>>>,
}

/// The TF-IDF base dimension the projection reduces from. The server
/// constructs `SvdEmbedding::new(reduced, vocabulary_size)` with a caller
/// -chosen vocabulary size; VecLite's collection config carries only the
/// output dimension, so the base vocabulary is fixed at the server's default
/// embedding width.
const BASE_VOCABULARY: usize = 512;

impl Svd {
    /// A fresh, unfitted SVD provider producing `dimension`-length vectors.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        Svd {
            dimension,
            tfidf: TfIdf::new(BASE_VOCABULARY),
            transformation: None,
        }
    }

    /// Build the projection matrix (server's `fit_svd`): hash-seed from the
    /// corpus, generate pseudo-random rows, orthogonalize them against each
    /// other (simplified Gram–Schmidt), and normalize. Deterministic for a
    /// given corpus — `DefaultHasher` uses fixed keys.
    fn fit_svd(&mut self, texts: &[&str]) -> Result<()> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        self.tfidf.fit(texts)?;
        let vocab_size = self.tfidf.dimension();
        let mut matrix: Vec<Vec<f32>> = Vec::with_capacity(self.dimension);

        let mut hasher = DefaultHasher::new();
        texts.hash(&mut hasher);
        let base_seed = hasher.finish();

        for i in 0..self.dimension {
            let mut vector = Vec::with_capacity(vocab_size);
            for j in 0..vocab_size {
                let seed = base_seed.wrapping_add((i as u64 * 1000) + j as u64);
                #[allow(clippy::cast_precision_loss)]
                let value = ((seed.wrapping_mul(1_103_515_245) % 65_536) as f32 / 32_768.0) - 1.0;
                vector.push(value);
            }
            // Orthogonalize against every previous row.
            for prev in &matrix {
                let dot: f32 = vector.iter().zip(prev.iter()).map(|(a, b)| a * b).sum();
                let norm_sq: f32 = prev.iter().map(|x| x * x).sum();
                if norm_sq > 0.0 {
                    let projection = dot / norm_sq;
                    for (v, p) in vector.iter_mut().zip(prev.iter()) {
                        *v -= projection * p;
                    }
                }
            }
            let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut vector {
                    *v /= norm;
                }
            }
            matrix.push(vector);
        }

        self.transformation = Some(matrix);
        Ok(())
    }
}

impl Embedder for Svd {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let Some(matrix) = &self.transformation else {
            // Unfitted: zero vector, replaced by the collection's refit
            // lifecycle (adaptation — the server errors here instead).
            return Ok(vec![0.0; self.dimension]);
        };
        let base = self.tfidf.embed(text)?;
        let mut out = vec![0.0f32; self.dimension];
        for (o, row) in out.iter_mut().zip(matrix.iter()) {
            for (b, m) in base.iter().zip(row.iter()) {
                *o += b * m;
            }
        }
        Ok(out)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn fit(&mut self, corpus: &[&str]) -> Result<()> {
        self.fit_svd(corpus)
    }

    // add_document: default no-op — the projection is a function of the whole
    // corpus, so incremental updates do not apply; `refit` rebuilds exactly.

    fn export_state(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self)
            .map_err(|e| crate::error::VecLiteError::Corrupt(format!("svd: export_state: {e}")))
    }

    fn import_state(&mut self, state: &[u8]) -> Result<()> {
        let restored: Svd = serde_json::from_slice(state)
            .map_err(|e| crate::error::VecLiteError::Corrupt(format!("svd: import_state: {e}")))?;
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Vec<&'static str> {
        vec![
            "cats are small furry animals that meow",
            "dogs are loyal furry animals that bark",
            "cars are fast vehicles with engines",
            "trains are long vehicles on rails",
        ]
    }

    #[test]
    fn fit_projects_and_ranks_similar_texts_closer() {
        let mut svd = Svd::new(8);
        svd.fit(&corpus()).unwrap_or_else(|e| panic!("{e}"));
        let cat = svd
            .embed("furry animals that meow")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cat.len(), 8);
        assert!(cat.iter().any(|&v| v != 0.0), "fitted embed is non-zero");

        let cos = |a: &[f32], b: &[f32]| {
            let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
            dot / (na * nb)
        };
        let animals = svd
            .embed("cats are small furry animals that meow")
            .unwrap_or_else(|e| panic!("{e}"));
        let vehicles = svd
            .embed("cars are fast vehicles with engines")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(cos(&cat, &animals) > cos(&cat, &vehicles));
    }

    #[test]
    fn unfitted_embeds_zero_and_state_round_trips() {
        let svd = Svd::new(4);
        assert_eq!(
            svd.embed("anything").unwrap_or_else(|e| panic!("{e}")),
            vec![0.0; 4]
        );

        let mut fitted = Svd::new(4);
        fitted.fit(&corpus()).unwrap_or_else(|e| panic!("{e}"));
        let state = fitted.export_state().unwrap_or_else(|e| panic!("{e}"));
        let mut restored = Svd::new(4);
        restored
            .import_state(&state)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fitted.embed("cats meow").unwrap_or_else(|e| panic!("{e}")),
            restored
                .embed("cats meow")
                .unwrap_or_else(|e| panic!("{e}"))
        );
    }

    #[test]
    fn deterministic_for_a_fixed_corpus() {
        let mut a = Svd::new(6);
        let mut b = Svd::new(6);
        a.fit(&corpus()).unwrap_or_else(|e| panic!("{e}"));
        b.fit(&corpus()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            a.embed("furry cats").unwrap_or_else(|e| panic!("{e}")),
            b.embed("furry cats").unwrap_or_else(|e| panic!("{e}"))
        );
    }
}
