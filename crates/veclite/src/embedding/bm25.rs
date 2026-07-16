//! BM25 sparse embedding provider (SPEC-005, default auto-embed provider).
//!
//! Vendored from `vectorizer/src/embedding/providers/bm25.rs` (ADR-0001):
//! the tokenizer, vocabulary construction, and BM25 scoring math are kept
//! byte-for-byte identical (server parity within 1e-5, EMB-002) — `k1 = 1.5`,
//! `b = 0.75`, `idf = ln((N - df + 0.5)/(df + 0.5) + 1)`. The server's
//! monitoring counters, rate-limited warnings, and hash-placeholder fallbacks
//! (EMB-001, excluded) are intentionally NOT ported: an out-of-vocabulary text
//! yields the honest (possibly empty) BM25 vector rather than a hash.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::Embedder;
use crate::error::{Result, VecLiteError};

/// BM25 provider state. The vocabulary maps the top-`dimension` terms (by corpus
/// frequency) to their output index; `doc_freq` holds each term's document
/// frequency for the IDF term.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25 {
    dimension: usize,
    vocabulary: HashMap<String, usize>,
    doc_freq: HashMap<String, usize>,
    doc_lengths: Vec<usize>,
    avg_doc_length: f32,
    total_docs: usize,
    k1: f32,
    b: f32,
}

impl Bm25 {
    /// A fresh BM25 provider with the server-parity parameters (`k1 = 1.5`,
    /// `b = 0.75`) and an empty vocabulary.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        Bm25 {
            dimension,
            vocabulary: HashMap::new(),
            doc_freq: HashMap::new(),
            doc_lengths: Vec::new(),
            avg_doc_length: 0.0,
            total_docs: 0,
            k1: 1.5,
            b: 0.75,
        }
    }

    /// Number of terms in the vocabulary.
    #[must_use]
    pub fn vocabulary_size(&self) -> usize {
        self.vocabulary.len()
    }

    /// Tokenize: lowercase, split on whitespace, trim non-alphanumeric edges,
    /// drop empties (identical to the server tokenizer).
    fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split_whitespace()
            .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Build the vocabulary and document statistics from a corpus (server's
    /// `build_vocabulary`): term/document-frequency counts, average document
    /// length, and the top-`dimension` terms by frequency (ties broken by term).
    fn build_vocabulary(&mut self, texts: &[&str]) {
        let mut word_counts: HashMap<String, usize> = HashMap::new();
        let mut doc_frequencies: HashMap<String, usize> = HashMap::new();

        for text in texts {
            let tokens = Self::tokenize(text);
            self.doc_lengths.push(tokens.len());

            let mut unique_terms = HashSet::new();
            for token in &tokens {
                *word_counts.entry(token.clone()).or_insert(0) += 1;
                unique_terms.insert(token.clone());
            }
            for term in unique_terms {
                *doc_frequencies.entry(term).or_insert(0) += 1;
            }
        }

        self.total_docs = texts.len();
        self.avg_doc_length = if self.total_docs == 0 {
            0.0
        } else {
            self.doc_lengths.iter().sum::<usize>() as f32 / self.total_docs as f32
        };

        // Sort by frequency (desc), ties by term (asc) — deterministic.
        let mut word_freq: Vec<(String, usize)> = word_counts.into_iter().collect();
        word_freq.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        for (i, (word, _)) in word_freq.into_iter().enumerate().take(self.dimension) {
            let df = *doc_frequencies.get(&word).unwrap_or(&0);
            self.vocabulary.insert(word.clone(), i);
            self.doc_freq.insert(word, df);
        }
    }

    /// Fold one document into the statistics incrementally (EMB-030,
    /// approximate): document count / average length update exactly; document
    /// frequencies bump for known terms; new terms append at the next free
    /// index while vocabulary space remains (existing indices never move, so
    /// stored vectors stay comparable). `fit` remains the exact recompute.
    fn add_document_inner(&mut self, text: &str) {
        let tokens = Self::tokenize(text);
        self.doc_lengths.push(tokens.len());
        self.total_docs += 1;
        #[allow(clippy::cast_precision_loss)]
        {
            self.avg_doc_length =
                self.doc_lengths.iter().sum::<usize>() as f32 / self.total_docs as f32;
        }
        let mut unique: Vec<String> = {
            let set: HashSet<String> = tokens.into_iter().collect();
            set.into_iter().collect()
        };
        unique.sort(); // deterministic append order for new terms
        for term in unique {
            if self.vocabulary.contains_key(&term) {
                *self.doc_freq.entry(term).or_insert(0) += 1;
            } else if self.vocabulary.len() < self.dimension {
                let idx = self.vocabulary.len();
                self.vocabulary.insert(term.clone(), idx);
                self.doc_freq.insert(term, 1);
            }
        }
    }

    /// BM25 score for one term (server-identical). `idf` uses the `+1` variant
    /// so it is never negative; `tf` is the saturating BM25 term-frequency.
    fn bm25_score(&self, term_freq: usize, doc_length: usize, doc_freq: usize) -> f32 {
        if doc_freq == 0 || self.avg_doc_length == 0.0 {
            return 0.0;
        }
        let idf =
            ((self.total_docs as f32 - doc_freq as f32 + 0.5) / (doc_freq as f32 + 0.5) + 1.0).ln();
        let tf = term_freq as f32 * (self.k1 + 1.0)
            / (term_freq as f32
                + self.k1 * (1.0 - self.b + self.b * doc_length as f32 / self.avg_doc_length));
        idf * tf
    }
}

impl Embedder for Bm25 {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let tokens = Self::tokenize(text);
        let doc_length = tokens.len();

        let mut term_freq: HashMap<String, usize> = HashMap::new();
        for token in tokens {
            *term_freq.entry(token).or_insert(0) += 1;
        }

        let mut embedding = vec![0.0f32; self.dimension];
        for (term, &vocab_index) in &self.vocabulary {
            if vocab_index >= self.dimension {
                continue;
            }
            let tf = *term_freq.get(term).unwrap_or(&0);
            if tf > 0 {
                let df = *self.doc_freq.get(term).unwrap_or(&0);
                embedding[vocab_index] = self.bm25_score(tf, doc_length, df);
            }
        }

        // L2-normalize (server parity). An all-out-of-vocabulary text stays a
        // zero vector — the honest "no lexical overlap" answer.
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

    fn fit(&mut self, corpus: &[&str]) -> Result<()> {
        // Full recompute: reset the trainable state, then rebuild from scratch
        // (EMB-031 exact semantics).
        self.vocabulary.clear();
        self.doc_freq.clear();
        self.doc_lengths.clear();
        self.avg_doc_length = 0.0;
        self.total_docs = 0;
        self.build_vocabulary(corpus);
        Ok(())
    }

    fn add_document(&mut self, text: &str) {
        self.add_document_inner(text);
    }

    fn export_state(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self)
            .map_err(|e| VecLiteError::Corrupt(format!("bm25: export_state: {e}")))
    }

    fn import_state(&mut self, state: &[u8]) -> Result<()> {
        let restored: Bm25 = serde_json::from_slice(state)
            .map_err(|e| VecLiteError::Corrupt(format!("bm25: import_state: {e}")))?;
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Vec<&'static str> {
        vec![
            "the quick brown fox jumps over the lazy dog",
            "a quick brown dog runs fast",
            "the lazy cat sleeps all day",
            "brown foxes and brown dogs are quick",
        ]
    }

    #[test]
    fn fit_builds_vocabulary_and_embeds_deterministically() {
        let mut bm = Bm25::new(64);
        bm.fit(&corpus()).unwrap_or_else(|e| panic!("{e}"));
        assert!(bm.vocabulary_size() > 0);

        // Deterministic: the same text embeds identically every call.
        let a = bm
            .embed("quick brown fox")
            .unwrap_or_else(|e| panic!("{e}"));
        let b = bm
            .embed("quick brown fox")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);

        // In-vocabulary text yields a unit vector.
        let norm = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "expected unit norm, got {norm}");
    }

    #[test]
    fn out_of_vocabulary_text_is_zero_vector() {
        let mut bm = Bm25::new(32);
        bm.fit(&corpus()).unwrap_or_else(|e| panic!("{e}"));
        let v = bm
            .embed("zzz qqq xyzzy plugh")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(v.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn similar_texts_score_higher_than_unrelated() {
        let mut bm = Bm25::new(64);
        bm.fit(&corpus()).unwrap_or_else(|e| panic!("{e}"));
        let cos =
            |a: &[f32], b: &[f32]| -> f32 { a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() };
        let q = bm
            .embed("quick brown dog")
            .unwrap_or_else(|e| panic!("{e}"));
        let near = bm
            .embed("a quick brown dog runs fast")
            .unwrap_or_else(|e| panic!("{e}"));
        let far = bm
            .embed("the lazy cat sleeps all day")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(cos(&q, &near) > cos(&q, &far));
    }

    #[test]
    fn export_import_round_trips_state() {
        let mut bm = Bm25::new(48);
        bm.fit(&corpus()).unwrap_or_else(|e| panic!("{e}"));
        let state = bm.export_state().unwrap_or_else(|e| panic!("{e}"));
        let q = bm
            .embed("quick brown fox")
            .unwrap_or_else(|e| panic!("{e}"));

        let mut restored = Bm25::new(1); // wrong dim on purpose; import overrides
        restored
            .import_state(&state)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(restored.dimension(), 48);
        assert_eq!(
            restored
                .embed("quick brown fox")
                .unwrap_or_else(|e| panic!("{e}")),
            q
        );
    }

    #[test]
    fn fit_with_empty_corpus_yields_zero_average_length_and_empty_embedding() {
        let mut bm = Bm25::new(16);
        bm.fit(&[]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(bm.vocabulary_size(), 0);
        assert_eq!(bm.avg_doc_length, 0.0);
        let v = bm
            .embed("anything at all")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(v.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn bm25_score_is_zero_when_untrained_or_term_absent_from_corpus() {
        // Untrained state: `avg_doc_length` is 0.0 before any `fit`.
        let untrained = Bm25::new(8);
        assert_eq!(untrained.bm25_score(1, 1, 1), 0.0);

        // Trained state, but the queried term has doc_freq == 0 (absent).
        let mut bm = Bm25::new(8);
        bm.fit(&["x y", "x z"]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(bm.bm25_score(1, 1, 0), 0.0);
    }

    #[test]
    fn embed_skips_vocabulary_entries_with_a_stale_out_of_range_index() {
        // A hand-edited/corrupted imported state can carry a vocabulary
        // index that no longer fits `dimension` (e.g. after shrinking it
        // out of band); `embed` must skip such entries instead of
        // panicking on out-of-bounds indexing.
        let mut vocabulary = HashMap::new();
        vocabulary.insert("cat".to_string(), 0);
        vocabulary.insert("dog".to_string(), 5); // out of range for dimension 2
        let mut doc_freq = HashMap::new();
        doc_freq.insert("cat".to_string(), 1);
        doc_freq.insert("dog".to_string(), 1);
        let bm = Bm25 {
            dimension: 2,
            vocabulary,
            doc_freq,
            doc_lengths: vec![2],
            avg_doc_length: 2.0,
            total_docs: 1,
            k1: 1.5,
            b: 0.75,
        };
        let v = bm.embed("cat dog").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v.len(), 2);
        assert!(v[0] > 0.0, "in-range term must still be scored");
        assert_eq!(v[1], 0.0, "index 5 was out of range and must be skipped");
    }

    #[test]
    fn bm25_idf_and_tf_match_the_formula() {
        // Two docs, one term "x" in both → df=2, N=2.
        let mut bm = Bm25::new(8);
        bm.fit(&["x y", "x z"]).unwrap_or_else(|e| panic!("{e}"));
        // idf = ln((2 - 2 + 0.5)/(2 + 0.5) + 1) = ln(0.2 + 1) = ln(1.2)
        let expected_idf = (0.5f32 / 2.5 + 1.0).ln();
        // For query "x": tf=1, doc_length=1, avg_doc_length=2.
        // tf_component = 1*(1.5+1)/(1 + 1.5*(1 - 0.75 + 0.75*1/2)) = 2.5/(1 + 1.5*0.625)
        let tf_component = 2.5f32 / (1.0 + 1.5 * (1.0 - 0.75 + 0.75 * 0.5));
        let expected = expected_idf * tf_component;
        let raw = bm.bm25_score(1, 1, 2);
        assert!(
            (raw - expected).abs() < 1e-6,
            "raw {raw} vs expected {expected}"
        );
    }
}
