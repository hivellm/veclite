//! In-memory payload indexes (SPEC-006 §3): `value → roaring bitmap of slots`
//! for `Keyword`/`Integer`/`Float` keys. They are **accelerators, not gates**
//! (FLT-022): `candidates` returns a *superset* of the matching slots for the
//! `must` clause (the caller always applies the full `Filter` afterward, so the
//! result set is identical with or without an index — FLT-031). The query-time
//! accelerator is wired only on native (wasm filters by scan, FLT-022), but this
//! module is portable — the seal path builds PIDX bitmaps and load harvests the
//! declarations on every target (roaring is pure Rust).
//!
//! Persistence is by rebuild: like the HNSW graph, indexes are reconstructed
//! from the loaded payloads on open rather than stored as PIDX segments.

use std::collections::{BTreeMap, HashMap};

use roaring::RoaringTreemap;
use serde_json::Value;

use super::{Condition, Filter, MatchValue};
use crate::options::PayloadIndexKind;

/// Total-order wrapper for `f64` index keys (BTreeMap needs `Ord`; NaN never
/// enters — payloads reject non-finite numbers via JSON, and only finite `as_f64`
/// values are inserted).
#[derive(Clone, Copy, PartialEq)]
struct OrdF64(f64);
impl Eq for OrdF64 {}
impl PartialOrd for OrdF64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrdF64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// One key's index; the variant matches the declared [`PayloadIndexKind`].
enum KeyIndex {
    Keyword(HashMap<String, RoaringTreemap>),
    Integer(BTreeMap<i64, RoaringTreemap>),
    Float(BTreeMap<OrdF64, RoaringTreemap>),
}

impl KeyIndex {
    fn empty(kind: PayloadIndexKind) -> Self {
        match kind {
            PayloadIndexKind::Keyword => KeyIndex::Keyword(HashMap::new()),
            PayloadIndexKind::Integer => KeyIndex::Integer(BTreeMap::new()),
            PayloadIndexKind::Float => KeyIndex::Float(BTreeMap::new()),
        }
    }

    fn add(&mut self, value: &Value, slot: u64) {
        match self {
            KeyIndex::Keyword(m) => {
                if let Some(s) = value.as_str() {
                    m.entry(s.to_owned()).or_default().insert(slot);
                }
            }
            KeyIndex::Integer(m) => {
                if let Some(i) = value.as_i64() {
                    m.entry(i).or_default().insert(slot);
                }
            }
            KeyIndex::Float(m) => {
                if let Some(f) = value.as_f64() {
                    m.entry(OrdF64(f)).or_default().insert(slot);
                }
            }
        }
    }

    fn remove(&mut self, value: &Value, slot: u64) {
        match self {
            KeyIndex::Keyword(m) => {
                if let Some(s) = value.as_str() {
                    if let Some(b) = m.get_mut(s) {
                        b.remove(slot);
                    }
                }
            }
            KeyIndex::Integer(m) => {
                if let Some(i) = value.as_i64() {
                    if let Some(b) = m.get_mut(&i) {
                        b.remove(slot);
                    }
                }
            }
            KeyIndex::Float(m) => {
                if let Some(f) = value.as_f64() {
                    if let Some(b) = m.get_mut(&OrdF64(f)) {
                        b.remove(slot);
                    }
                }
            }
        }
    }

    /// All slots that have any indexed value for this key (an `Exists` answer).
    fn all_slots(&self) -> RoaringTreemap {
        let mut out = RoaringTreemap::new();
        match self {
            KeyIndex::Keyword(m) => m.values().for_each(|b| out |= b),
            KeyIndex::Integer(m) => m.values().for_each(|b| out |= b),
            KeyIndex::Float(m) => m.values().for_each(|b| out |= b),
        }
        out
    }
}

/// The set of payload indexes declared for a collection.
pub(crate) struct PayloadIndexes {
    by_key: HashMap<String, KeyIndex>,
}

/// One sealed posting value (SPEC-002 §3.1 PIDX body), typed by the declared
/// kind.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PostingValue {
    Keyword(String),
    Integer(i64),
    Float(f64),
}

impl PayloadIndexes {
    /// Build empty indexes for the declared `(key, kind)` set (CONFIG-derived).
    pub(crate) fn new(declared: &[(String, PayloadIndexKind)]) -> Self {
        let by_key = declared
            .iter()
            .map(|(k, kind)| (k.clone(), KeyIndex::empty(*kind)))
            .collect();
        PayloadIndexes { by_key }
    }

    /// Declare one more index at runtime (FLT-020 late creation). Returns
    /// `false` when `key` is already declared (the caller decides whether a
    /// same-kind redeclare is an idempotent no-op or a kind conflict).
    pub(crate) fn declare(&mut self, key: &str, kind: PayloadIndexKind) -> bool {
        if self.by_key.contains_key(key) {
            return false;
        }
        self.by_key.insert(key.to_owned(), KeyIndex::empty(kind));
        true
    }

    /// The declared kind for `key`, if indexed.
    pub(crate) fn kind_of(&self, key: &str) -> Option<PayloadIndexKind> {
        self.by_key.get(key).map(|idx| match idx {
            KeyIndex::Keyword(_) => PayloadIndexKind::Keyword,
            KeyIndex::Integer(_) => PayloadIndexKind::Integer,
            KeyIndex::Float(_) => PayloadIndexKind::Float,
        })
    }

    /// Every declared `(key, kind)`, sorted by key — the deterministic order
    /// PIDX segments are sealed in (STG-041 gives same-rank segments append
    /// order, so the writer fixes it).
    pub(crate) fn declared(&self) -> Vec<(String, PayloadIndexKind)> {
        let mut out: Vec<(String, PayloadIndexKind)> = self
            .by_key
            .keys()
            .map(|k| {
                (
                    k.clone(),
                    self.kind_of(k).unwrap_or(PayloadIndexKind::Keyword),
                )
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The sealed postings of one key's index: `(value, sorted slot list)` in
    /// ascending value order (SPEC-002 §3.1 PIDX body). `None` if undeclared.
    pub(crate) fn postings(&self, key: &str) -> Option<Vec<(PostingValue, Vec<u64>)>> {
        let idx = self.by_key.get(key)?;
        Some(match idx {
            KeyIndex::Keyword(m) => {
                let mut entries: Vec<_> = m.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                entries
                    .into_iter()
                    .map(|(v, b)| (PostingValue::Keyword(v.clone()), b.iter().collect()))
                    .collect()
            }
            KeyIndex::Integer(m) => m
                .iter()
                .map(|(v, b)| (PostingValue::Integer(*v), b.iter().collect()))
                .collect(),
            KeyIndex::Float(m) => m
                .iter()
                .map(|(v, b)| (PostingValue::Float(v.0), b.iter().collect()))
                .collect(),
        })
    }

    /// Add a point's payload to every declared index.
    pub(crate) fn insert(&mut self, slot: u64, payload: Option<&Value>) {
        if self.by_key.is_empty() {
            return;
        }
        let Some(obj) = payload.and_then(Value::as_object) else {
            return;
        };
        for (key, idx) in &mut self.by_key {
            if let Some(v) = obj.get(key) {
                idx.add(v, slot);
            }
        }
    }

    /// Remove a tombstoned/replaced point's payload from every index.
    pub(crate) fn remove(&mut self, slot: u64, payload: Option<&Value>) {
        if self.by_key.is_empty() {
            return;
        }
        let Some(obj) = payload.and_then(Value::as_object) else {
            return;
        };
        for (key, idx) in &mut self.by_key {
            if let Some(v) = obj.get(key) {
                idx.remove(v, slot);
            }
        }
    }

    /// A superset of the slots matching `filter.must`, from the index-answerable
    /// conditions only, or `None` when no `must` condition is index-answerable
    /// (so the caller should post-filter without acceleration). The caller MUST
    /// still apply the full filter to each candidate (FLT-031).
    pub(crate) fn candidates(&self, filter: &Filter) -> Option<RoaringTreemap> {
        let mut acc: Option<RoaringTreemap> = None;
        for cond in &filter.must {
            if let Some(bitmap) = self.answer(cond) {
                acc = Some(match acc {
                    None => bitmap,
                    Some(a) => a & bitmap,
                });
            }
        }
        acc
    }

    /// The slot bitmap a single condition selects, if its key is indexed and the
    /// condition kind is answerable; `None` otherwise (fall back to scan).
    fn answer(&self, cond: &Condition) -> Option<RoaringTreemap> {
        match cond {
            Condition::Eq { key, value } => {
                let idx = self.by_key.get(key)?;
                Some(eq_bitmap(idx, value))
            }
            Condition::In { key, values } => {
                let idx = self.by_key.get(key)?;
                let mut out = RoaringTreemap::new();
                for v in values {
                    out |= eq_bitmap(idx, v);
                }
                Some(out)
            }
            Condition::Range { key, range } => {
                let idx = self.by_key.get(key)?;
                range_bitmap(idx, range)
            }
            Condition::Exists { key } => self.by_key.get(key).map(KeyIndex::all_slots),
            Condition::Nested(_) => None,
        }
    }
}

/// Exact-match bitmap for a value within one key's index (empty if the kind and
/// value type disagree or the value is absent).
fn eq_bitmap(idx: &KeyIndex, value: &MatchValue) -> RoaringTreemap {
    match (idx, value) {
        (KeyIndex::Keyword(m), MatchValue::Keyword(s)) => m.get(s).cloned().unwrap_or_default(),
        (KeyIndex::Integer(m), MatchValue::Integer(i)) => m.get(i).cloned().unwrap_or_default(),
        (KeyIndex::Float(m), MatchValue::Integer(i)) => {
            m.get(&OrdF64(*i as f64)).cloned().unwrap_or_default()
        }
        _ => RoaringTreemap::new(),
    }
}

/// Range bitmap over an integer or float index; `None` for a keyword index
/// (range is not answerable by it — fall back to scan).
fn range_bitmap(idx: &KeyIndex, range: &super::Range) -> Option<RoaringTreemap> {
    let mut out = RoaringTreemap::new();
    match idx {
        KeyIndex::Integer(m) => {
            for (k, b) in m {
                if range.contains(*k as f64) {
                    out |= b;
                }
            }
            Some(out)
        }
        KeyIndex::Float(m) => {
            for (k, b) in m {
                if range.contains(k.0) {
                    out |= b;
                }
            }
            Some(out)
        }
        KeyIndex::Keyword(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{Condition, Filter, Range};
    use serde_json::json;

    fn indexes() -> PayloadIndexes {
        PayloadIndexes::new(&[
            ("lang".to_owned(), PayloadIndexKind::Keyword),
            ("year".to_owned(), PayloadIndexKind::Integer),
        ])
    }

    fn build() -> PayloadIndexes {
        let mut ix = indexes();
        ix.insert(0, Some(&json!({"lang": "en", "year": 2024})));
        ix.insert(1, Some(&json!({"lang": "pt", "year": 2020})));
        ix.insert(2, Some(&json!({"lang": "en", "year": 2018})));
        ix
    }

    fn slots(b: &RoaringTreemap) -> Vec<u64> {
        b.iter().collect()
    }

    #[test]
    fn eq_candidates() {
        let ix = build();
        let f = Filter::new().must(Condition::eq("lang", "en"));
        assert_eq!(
            slots(&ix.candidates(&f).unwrap_or_else(|| panic!("indexed"))),
            vec![0, 2]
        );
    }

    #[test]
    fn range_and_intersection() {
        let ix = build();
        // lang=en AND year>=2021 → only slot 0
        let f = Filter::new()
            .must(Condition::eq("lang", "en"))
            .must(Condition::range("year", Range::new().gte(2021.0)));
        assert_eq!(
            slots(&ix.candidates(&f).unwrap_or_else(|| panic!("indexed"))),
            vec![0]
        );
    }

    #[test]
    fn unindexed_key_returns_none() {
        let ix = build();
        let f = Filter::new().must(Condition::eq("author", "x"));
        assert!(ix.candidates(&f).is_none());
    }

    #[test]
    fn remove_updates_candidates() {
        let mut ix = build();
        ix.remove(0, Some(&json!({"lang": "en", "year": 2024})));
        let f = Filter::new().must(Condition::eq("lang", "en"));
        assert_eq!(
            slots(&ix.candidates(&f).unwrap_or_else(|| panic!("indexed"))),
            vec![2]
        );
    }

    #[test]
    fn ordf64_total_order() {
        // partial_cmp/cmp: total order incl. the operators BTreeMap never calls.
        assert!(OrdF64(1.0) < OrdF64(2.0));
        assert!(OrdF64(2.0) > OrdF64(1.0));
        assert_eq!(OrdF64(1.5).cmp(&OrdF64(1.5)), std::cmp::Ordering::Equal);
        assert!(OrdF64(1.5) == OrdF64(1.5));
    }

    fn float_ix() -> PayloadIndexes {
        let mut ix = PayloadIndexes::new(&[("price".to_owned(), PayloadIndexKind::Float)]);
        ix.insert(0, Some(&json!({"price": 9.99})));
        ix.insert(1, Some(&json!({"price": 19.5})));
        ix.insert(2, Some(&json!({"price": 4.25})));
        ix
    }

    #[test]
    fn float_range_candidates() {
        let ix = float_ix();
        // price in [5, 20) → slots 0 (9.99) and 1 (19.5), not 2 (4.25).
        let f = Filter::new().must(Condition::range("price", Range::new().gte(5.0).lt(20.0)));
        assert_eq!(
            slots(&ix.candidates(&f).unwrap_or_else(|| panic!("indexed"))),
            vec![0, 1]
        );
    }

    #[test]
    fn float_remove_and_exists() {
        let mut ix = float_ix();
        ix.remove(1, Some(&json!({"price": 19.5})));
        // Exists answers via all_slots over the remaining float entries.
        let f = Filter::new().must(Condition::exists("price"));
        assert_eq!(
            slots(&ix.candidates(&f).unwrap_or_else(|| panic!("indexed"))),
            vec![0, 2]
        );
    }

    #[test]
    fn float_eq_from_integer_match_value() {
        // A whole-number float indexed, queried with an integer literal.
        let mut ix = PayloadIndexes::new(&[("n".to_owned(), PayloadIndexKind::Float)]);
        ix.insert(7, Some(&json!({"n": 3.0})));
        let f = Filter::new().must(Condition::eq("n", 3i64));
        assert_eq!(
            slots(&ix.candidates(&f).unwrap_or_else(|| panic!("indexed"))),
            vec![7]
        );
    }

    #[test]
    fn in_condition_unions_matches() {
        let ix = build();
        let f = Filter::new().must(Condition::in_("year", vec![2024i64, 2018]));
        assert_eq!(
            slots(&ix.candidates(&f).unwrap_or_else(|| panic!("indexed"))),
            vec![0, 2]
        );
    }

    #[test]
    fn exists_over_keyword_index() {
        let ix = build();
        let f = Filter::new().must(Condition::exists("lang"));
        assert_eq!(
            slots(&ix.candidates(&f).unwrap_or_else(|| panic!("indexed"))),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn keyword_range_is_not_index_answerable() {
        // A keyword index cannot answer a Range → `answer` returns None, so a
        // must made only of it yields no candidates (scan fallback).
        let ix = build();
        let f = Filter::new().must(Condition::range("lang", Range::new().gte(1.0)));
        assert!(ix.candidates(&f).is_none());
    }

    #[test]
    fn nested_condition_is_not_index_answerable() {
        let ix = build();
        let nested = Filter::new().must(Condition::eq("lang", "en"));
        let f = Filter::new().must(Condition::Nested(Box::new(nested)));
        assert!(ix.candidates(&f).is_none());
    }

    #[test]
    fn declare_kind_and_postings() {
        let mut ix = PayloadIndexes::new(&[]);
        assert!(ix.declare("p", PayloadIndexKind::Float));
        assert!(!ix.declare("p", PayloadIndexKind::Float)); // already declared
        assert_eq!(ix.kind_of("p"), Some(PayloadIndexKind::Float));
        assert_eq!(ix.kind_of("absent"), None);

        ix.insert(0, Some(&json!({"p": 2.5})));
        ix.insert(1, Some(&json!({"p": 1.5})));
        let postings = ix.postings("p").unwrap_or_else(|| panic!("declared"));
        // Ascending float order: 1.5 (slot 1) before 2.5 (slot 0).
        assert_eq!(
            postings,
            vec![
                (PostingValue::Float(1.5), vec![1]),
                (PostingValue::Float(2.5), vec![0]),
            ]
        );
        assert!(ix.postings("absent").is_none());
        assert_eq!(
            ix.declared(),
            vec![("p".to_owned(), PayloadIndexKind::Float)]
        );
    }

    #[test]
    fn insert_and_remove_are_noops_without_indexes() {
        // No declared keys → insert/remove short-circuit; a non-object payload
        // is ignored even when keys are declared.
        let mut none = PayloadIndexes::new(&[]);
        none.insert(0, Some(&json!({"a": 1})));
        none.remove(0, Some(&json!({"a": 1})));
        assert!(
            none.candidates(&Filter::new().must(Condition::eq("a", 1i64)))
                .is_none()
        );

        let mut ix = indexes();
        ix.insert(0, Some(&json!("not-an-object")));
        ix.insert(1, None);
        let f = Filter::new().must(Condition::exists("lang"));
        assert!(
            ix.candidates(&f)
                .unwrap_or_else(|| panic!("indexed"))
                .is_empty()
        );
    }
}
