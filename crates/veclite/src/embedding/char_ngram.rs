//! Character n-gram sparse embedding provider (SPEC-005), typo-tolerant lexical.
//! Vendored from `vectorizer/src/embedding/providers/char_ngram.rs` (ADR-0001):
//! the n-gram extraction and top-`dimension` vocabulary are kept identical. The
//! server's hash fallback (EMB-001) is not ported. Default `n = 3` (trigrams).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::Embedder;
use crate::error::{Result, VecLiteError};

/// Default character n-gram size (trigrams).
pub const DEFAULT_N: usize = 3;

/// Character n-gram provider state: the n-gram size and its top-`dimension`
/// n-gram vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharNgram {
    dimension: usize,
    n: usize,
    ngram_map: HashMap<String, usize>,
}

impl CharNgram {
    /// A fresh provider with n-gram size `n` and an empty vocabulary.
    #[must_use]
    pub fn new(dimension: usize, n: usize) -> Self {
        CharNgram {
            dimension,
            n: n.max(1),
            ngram_map: HashMap::new(),
        }
    }

    fn extract_ngrams(&self, text: &str) -> Vec<String> {
        let chars: Vec<char> = text.to_lowercase().chars().collect();
        if chars.len() < self.n {
            return vec![chars.iter().collect()];
        }
        (0..=chars.len() - self.n)
            .map(|i| chars[i..i + self.n].iter().collect())
            .collect()
    }

    fn build_vocabulary(&mut self, texts: &[&str]) {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for text in texts {
            for ngram in self.extract_ngrams(text) {
                *counts.entry(ngram).or_insert(0) += 1;
            }
        }
        let mut freq: Vec<(String, usize)> = counts.into_iter().collect();
        freq.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        self.ngram_map.clear();
        for (i, (ngram, _)) in freq.iter().take(self.dimension).enumerate() {
            self.ngram_map.insert(ngram.clone(), i);
        }
    }
}

impl Embedder for CharNgram {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut embedding = vec![0.0f32; self.dimension];
        for ngram in self.extract_ngrams(text) {
            if let Some(&idx) = self.ngram_map.get(&ngram) {
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
        // Incremental (EMB-030, approximate): append unseen n-grams at the
        // next free index while vocabulary space remains; indices never move.
        let mut unique: Vec<String> = {
            let set: std::collections::HashSet<String> =
                self.extract_ngrams(text).into_iter().collect();
            set.into_iter().collect()
        };
        unique.sort();
        for ngram in unique {
            if !self.ngram_map.contains_key(&ngram) && self.ngram_map.len() < self.dimension {
                let idx = self.ngram_map.len();
                self.ngram_map.insert(ngram, idx);
            }
        }
    }

    fn fit(&mut self, corpus: &[&str]) -> Result<()> {
        self.build_vocabulary(corpus);
        Ok(())
    }

    fn export_state(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self)
            .map_err(|e| VecLiteError::Corrupt(format!("char_ngram: export: {e}")))
    }

    fn import_state(&mut self, state: &[u8]) -> Result<()> {
        *self = serde_json::from_slice(state)
            .map_err(|e| VecLiteError::Corrupt(format!("char_ngram: import: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typo_tolerant_overlap() {
        let mut c = CharNgram::new(128, DEFAULT_N);
        c.fit(&["hello world", "goodbye world"])
            .unwrap_or_else(|e| panic!("{e}"));
        let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let base = c.embed("hello").unwrap_or_else(|e| panic!("{e}"));
        let typo = c.embed("helo").unwrap_or_else(|e| panic!("{e}")); // shares "hel"
        let other = c.embed("world").unwrap_or_else(|e| panic!("{e}"));
        assert!(cos(&base, &typo) > cos(&base, &other));
    }

    #[test]
    fn round_trips_state() {
        let mut c = CharNgram::new(64, 3);
        c.fit(&["abcdef", "abcxyz"])
            .unwrap_or_else(|e| panic!("{e}"));
        let v = c.embed("abc").unwrap_or_else(|e| panic!("{e}"));
        let state = c.export_state().unwrap_or_else(|e| panic!("{e}"));
        let mut back = CharNgram::new(1, 1);
        back.import_state(&state).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.embed("abc").unwrap_or_else(|e| panic!("{e}")), v);
    }
}
