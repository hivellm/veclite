//! Bag-of-words sparse embedding provider (SPEC-005). Vendored from
//! `vectorizer/src/embedding/providers/bag_of_words.rs` (ADR-0001): the
//! tokenizer and top-`dimension` term-count vocabulary are kept identical. The
//! server's hash fallback (EMB-001) is not ported.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::Embedder;
use crate::error::{Result, VecLiteError};

/// Bag-of-words provider state: the top terms by frequency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BagOfWords {
    dimension: usize,
    vocabulary: HashMap<String, usize>,
}

impl BagOfWords {
    /// A fresh bag-of-words provider with an empty vocabulary.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        BagOfWords {
            dimension,
            vocabulary: HashMap::new(),
        }
    }

    fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|w| !w.is_empty())
            .collect()
    }

    fn build_vocabulary(&mut self, texts: &[&str]) {
        let mut word_counts: HashMap<String, usize> = HashMap::new();
        for text in texts {
            for word in Self::tokenize(text) {
                *word_counts.entry(word).or_insert(0) += 1;
            }
        }
        let mut word_freq: Vec<(String, usize)> = word_counts.into_iter().collect();
        word_freq.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        self.vocabulary.clear();
        for (i, (word, _)) in word_freq.iter().take(self.dimension).enumerate() {
            self.vocabulary.insert(word.clone(), i);
        }
    }
}

impl Embedder for BagOfWords {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut embedding = vec![0.0f32; self.dimension];
        for word in Self::tokenize(text) {
            if let Some(&idx) = self.vocabulary.get(&word) {
                embedding[idx] += 1.0;
            }
        }
        let norm = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut embedding {
                *v /= norm;
            }
        }
        Ok(embedding)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn add_document(&mut self, text: &str) {
        // Incremental (EMB-030, approximate): append unseen terms at the next
        // free index while vocabulary space remains; indices never move.
        let mut unique: Vec<String> = {
            let set: std::collections::HashSet<String> = Self::tokenize(text).into_iter().collect();
            set.into_iter().collect()
        };
        unique.sort();
        for word in unique {
            if !self.vocabulary.contains_key(&word) && self.vocabulary.len() < self.dimension {
                let idx = self.vocabulary.len();
                self.vocabulary.insert(word, idx);
            }
        }
    }

    fn fit(&mut self, corpus: &[&str]) -> Result<()> {
        self.build_vocabulary(corpus);
        Ok(())
    }

    fn export_state(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| VecLiteError::Corrupt(format!("bow: export: {e}")))
    }

    fn import_state(&mut self, state: &[u8]) -> Result<()> {
        *self = serde_json::from_slice(state)
            .map_err(|e| VecLiteError::Corrupt(format!("bow: import: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_of_unknown_terms_yields_zero_vector_without_dividing_by_zero() {
        let mut b = BagOfWords::new(16);
        b.fit(&["alpha beta gamma", "beta gamma delta"])
            .unwrap_or_else(|e| panic!("{e}"));
        let v = b.embed("zzz qqq unknown").unwrap_or_else(|e| panic!("{e}"));
        assert!(v.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn dimension_reports_the_configured_size() {
        let b = BagOfWords::new(24);
        assert_eq!(b.dimension(), 24);
    }

    #[test]
    fn fit_embed_and_round_trip() {
        let mut b = BagOfWords::new(16);
        b.fit(&["alpha beta gamma", "beta gamma delta"])
            .unwrap_or_else(|e| panic!("{e}"));
        let v = b.embed("beta gamma").unwrap_or_else(|e| panic!("{e}"));
        assert!(v.iter().any(|&x| x != 0.0));
        let state = b.export_state().unwrap_or_else(|e| panic!("{e}"));
        let mut back = BagOfWords::new(1);
        back.import_state(&state).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            back.embed("beta gamma").unwrap_or_else(|e| panic!("{e}")),
            v
        );
    }
}
