//! TF-IDF sparse embedding provider (SPEC-005). Vendored from
//! `vectorizer/src/embedding/providers/tfidf.rs` (ADR-0001): tokenizer,
//! top-`dimension` vocabulary by `tf·idf`, and `idf = ln(N/df).max(0)` kept
//! identical. The server's hash fallback (EMB-001) is not ported.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::Embedder;
use crate::error::{Result, VecLiteError};

/// TF-IDF provider state: the top terms and their IDF weights. The
/// `doc_frequencies`/`total_docs` tables back incremental updates (EMB-030);
/// they default to empty when importing pre-3f state (EMB-010 compatibility —
/// such state stays exact until the next `refit`, it just cannot update
/// incrementally).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TfIdf {
    dimension: usize,
    vocabulary: HashMap<String, usize>,
    idf_weights: Vec<f32>,
    #[serde(default)]
    doc_frequencies: HashMap<String, usize>,
    #[serde(default)]
    total_docs: usize,
}

impl TfIdf {
    /// A fresh TF-IDF provider with an empty vocabulary.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        TfIdf {
            dimension,
            vocabulary: HashMap::new(),
            idf_weights: Vec::new(),
            doc_frequencies: HashMap::new(),
            total_docs: 0,
        }
    }

    /// Fold one document in incrementally (EMB-030, approximate): bump
    /// document frequencies, append new terms while space remains (indices
    /// never move), and refresh every known term's IDF for the new `N`.
    #[allow(clippy::cast_precision_loss)]
    fn add_document_inner(&mut self, text: &str) {
        self.total_docs += 1;
        let mut unique: Vec<String> = {
            let set: HashSet<String> = Self::tokenize(text).into_iter().collect();
            set.into_iter().collect()
        };
        unique.sort(); // deterministic append order
        for word in unique {
            *self.doc_frequencies.entry(word.clone()).or_insert(0) += 1;
            if !self.vocabulary.contains_key(&word) && self.vocabulary.len() < self.dimension {
                let idx = self.vocabulary.len();
                self.vocabulary.insert(word, idx);
                self.idf_weights.push(0.0); // refreshed below
            }
        }
        // Refresh IDF for every term whose document frequency is tracked;
        // terms from legacy imports (no df table) keep their fitted weight.
        let n = self.total_docs as f32;
        for (word, &idx) in &self.vocabulary {
            if let Some(&df) = self.doc_frequencies.get(word) {
                if let Some(w) = self.idf_weights.get_mut(idx) {
                    *w = (n / df as f32).ln().max(0.0);
                }
            }
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
        let mut doc_frequencies: HashMap<String, usize> = HashMap::new();
        for text in texts {
            let mut seen = HashSet::new();
            for word in Self::tokenize(text) {
                *word_counts.entry(word.clone()).or_insert(0) += 1;
                if seen.insert(word.clone()) {
                    *doc_frequencies.entry(word).or_insert(0) += 1;
                }
            }
        }
        let total_docs = texts.len() as f32;

        let mut scored: Vec<(String, f32)> = doc_frequencies
            .iter()
            .map(|(word, &df)| {
                let tf_count = *word_counts.get(word).unwrap_or(&0) as f32;
                let idf = if df > 0 {
                    (total_docs / df as f32).ln().max(0.0)
                } else {
                    0.0
                };
                (word.clone(), tf_count * idf)
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });

        self.vocabulary.clear();
        self.idf_weights.clear();
        for (i, (word, _)) in scored.iter().take(self.dimension).enumerate() {
            self.vocabulary.insert(word.clone(), i);
            let df = *doc_frequencies.get(word).unwrap_or(&1) as f32;
            self.idf_weights.push((total_docs / df).ln().max(0.0));
        }
        // Keep the incremental tables in sync with the fitted state (EMB-030).
        self.doc_frequencies = doc_frequencies;
        self.total_docs = texts.len();
    }

    /// Term frequency (count / total) for the tokens of `text`.
    fn compute_tf(text: &str) -> HashMap<String, f32> {
        let words = Self::tokenize(text);
        let total = words.len() as f32;
        let mut counts: HashMap<String, usize> = HashMap::new();
        for w in words {
            *counts.entry(w).or_insert(0) += 1;
        }
        counts
            .into_iter()
            .map(|(w, c)| (w, c as f32 / total))
            .collect()
    }
}

impl Embedder for TfIdf {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut embedding = vec![0.0f32; self.dimension];
        for (word, tf) in Self::compute_tf(text) {
            if let Some(&idx) = self.vocabulary.get(&word) {
                if idx < self.dimension {
                    let idf = self.idf_weights.get(idx).copied().unwrap_or(1.0);
                    embedding[idx] = tf * idf;
                }
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
        self.add_document_inner(text);
    }

    fn fit(&mut self, corpus: &[&str]) -> Result<()> {
        self.build_vocabulary(corpus);
        Ok(())
    }

    fn export_state(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| VecLiteError::Corrupt(format!("tfidf: export: {e}")))
    }

    fn import_state(&mut self, state: &[u8]) -> Result<()> {
        *self = serde_json::from_slice(state)
            .map_err(|e| VecLiteError::Corrupt(format!("tfidf: import: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_embed_and_round_trip() {
        let mut t = TfIdf::new(32);
        t.fit(&[
            "the quick brown fox",
            "the lazy brown dog",
            "quick brown foxes",
        ])
        .unwrap_or_else(|e| panic!("{e}"));
        let v = t.embed("quick brown").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.len(), 32);
        assert!(v.iter().any(|&x| x != 0.0));
        let state = t.export_state().unwrap_or_else(|e| panic!("{e}"));
        let mut back = TfIdf::new(1);
        back.import_state(&state).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            back.embed("quick brown").unwrap_or_else(|e| panic!("{e}")),
            v
        );
    }
}
