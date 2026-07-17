//! Payload filters (SPEC-006): the Qdrant-style filter model, its server-parity
//! evaluation semantics (FLT-010/011), and JSON (de)serialization for portable
//! filter documents (SPEC-013). Filtering addresses **top-level** payload keys
//! only in v1; geo conditions and nested-path keys are rejected with
//! `InvalidArgument` (FLT-012), never silently ignored.
//!
//! Evaluation is pure (`Filter::matches` over a `serde_json::Value` payload);
//! index acceleration lives in [`index`], and the search integration is in
//! `collection`.

// Portable (roaring is pure Rust): the query-time accelerator is native-only
// (wired in `collection`), but the seal path builds PIDX bitmaps and load
// harvests declarations on every target, so the module itself is all-targets.
// On wasm only the build/harvest entry points are reached — the lookup methods
// (candidates/answer/…) are dead there, so silence dead-code on that target.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) mod index;

use serde_json::Value;

use crate::error::{Result, VecLiteError};

/// A value an `Eq`/`In` condition compares against: a string (keyword), an
/// integer, or a boolean (SPEC-006 §2).
#[derive(Clone, Debug, PartialEq)]
pub enum MatchValue {
    /// Exact, case-sensitive string match.
    Keyword(String),
    /// Integer match (JSON number equality — an integer stored as `2024` or
    /// `2024.0` both match `Integer(2024)`).
    Integer(i64),
    /// Exact boolean match.
    Boolean(bool),
}

impl From<&str> for MatchValue {
    fn from(s: &str) -> Self {
        MatchValue::Keyword(s.to_owned())
    }
}
impl From<String> for MatchValue {
    fn from(s: String) -> Self {
        MatchValue::Keyword(s)
    }
}
impl From<i64> for MatchValue {
    fn from(i: i64) -> Self {
        MatchValue::Integer(i)
    }
}
impl From<bool> for MatchValue {
    fn from(b: bool) -> Self {
        MatchValue::Boolean(b)
    }
}

impl MatchValue {
    /// Whether a stored payload value equals this match value (FLT-011).
    fn matches_stored(&self, stored: &Value) -> bool {
        match self {
            MatchValue::Keyword(s) => stored.as_str() == Some(s.as_str()),
            MatchValue::Boolean(b) => stored.as_bool() == Some(*b),
            // JSON number equality: integer or float representation.
            MatchValue::Integer(i) => {
                stored.as_i64() == Some(*i) || stored.as_f64() == Some(*i as f64)
            }
        }
    }

    fn from_json(v: &Value) -> Result<MatchValue> {
        match v {
            Value::String(s) => Ok(MatchValue::Keyword(s.clone())),
            Value::Bool(b) => Ok(MatchValue::Boolean(*b)),
            Value::Number(n) => n
                .as_i64()
                .map(MatchValue::Integer)
                .ok_or_else(|| unsupported("float match value (use a range condition)")),
            other => Err(unsupported(&format!("match value {other}"))),
        }
    }
}

/// Numeric range bounds for a `Range` condition (SPEC-006 §2). Any subset of the
/// four bounds may be set; an unset bound is unbounded on that side.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Range {
    /// Strictly greater than.
    pub gt: Option<f64>,
    /// Greater than or equal.
    pub gte: Option<f64>,
    /// Strictly less than.
    pub lt: Option<f64>,
    /// Less than or equal.
    pub lte: Option<f64>,
}

impl Range {
    /// An empty (unbounded) range.
    #[must_use]
    pub fn new() -> Self {
        Range::default()
    }
    /// Set the strict lower bound.
    #[must_use]
    pub fn gt(mut self, v: f64) -> Self {
        self.gt = Some(v);
        self
    }
    /// Set the inclusive lower bound.
    #[must_use]
    pub fn gte(mut self, v: f64) -> Self {
        self.gte = Some(v);
        self
    }
    /// Set the strict upper bound.
    #[must_use]
    pub fn lt(mut self, v: f64) -> Self {
        self.lt = Some(v);
        self
    }
    /// Set the inclusive upper bound.
    #[must_use]
    pub fn lte(mut self, v: f64) -> Self {
        self.lte = Some(v);
        self
    }

    fn contains(&self, x: f64) -> bool {
        self.gt.is_none_or(|b| x > b)
            && self.gte.is_none_or(|b| x >= b)
            && self.lt.is_none_or(|b| x < b)
            && self.lte.is_none_or(|b| x <= b)
    }
}

/// One filter condition over a top-level payload key (SPEC-006 §2).
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// Exact match on a keyword/integer/boolean value.
    Eq {
        /// Top-level payload key.
        key: String,
        /// Value to match.
        value: MatchValue,
    },
    /// Match any of the listed values.
    In {
        /// Top-level payload key.
        key: String,
        /// Candidate values (matches if the stored value equals any).
        values: Vec<MatchValue>,
    },
    /// Numeric range over the stored value.
    Range {
        /// Top-level payload key.
        key: String,
        /// Range bounds.
        range: Range,
    },
    /// The key is present in the payload (including a JSON `null` value).
    Exists {
        /// Top-level payload key.
        key: String,
    },
    /// A nested boolean composition of clauses.
    Nested(Box<Filter>),
}

impl Condition {
    /// Exact-match condition.
    #[must_use]
    pub fn eq(key: impl Into<String>, value: impl Into<MatchValue>) -> Condition {
        Condition::Eq {
            key: key.into(),
            value: value.into(),
        }
    }
    /// Set-membership condition.
    #[must_use]
    pub fn in_<V: Into<MatchValue>>(key: impl Into<String>, values: Vec<V>) -> Condition {
        Condition::In {
            key: key.into(),
            values: values.into_iter().map(Into::into).collect(),
        }
    }
    /// Numeric range condition.
    #[must_use]
    pub fn range(key: impl Into<String>, range: Range) -> Condition {
        Condition::Range {
            key: key.into(),
            range,
        }
    }
    /// Key-presence condition.
    #[must_use]
    pub fn exists(key: impl Into<String>) -> Condition {
        Condition::Exists { key: key.into() }
    }
    /// Nested boolean composition.
    #[must_use]
    pub fn nested(filter: Filter) -> Condition {
        Condition::Nested(Box::new(filter))
    }

    /// The top-level key this condition addresses, if any (`Nested` has none).
    fn key(&self) -> Option<&str> {
        match self {
            Condition::Eq { key, .. }
            | Condition::In { key, .. }
            | Condition::Range { key, .. }
            | Condition::Exists { key } => Some(key),
            Condition::Nested(_) => None,
        }
    }

    /// Evaluate against a payload object (FLT-011). A missing key never matches
    /// except `Exists`, which is exactly the presence test.
    fn matches(&self, payload: Option<&Value>) -> bool {
        match self {
            Condition::Eq { key, value } => {
                lookup(payload, key).is_some_and(|stored| value.matches_stored(stored))
            }
            Condition::In { key, values } => lookup(payload, key)
                .is_some_and(|stored| values.iter().any(|v| v.matches_stored(stored))),
            Condition::Range { key, range } => lookup(payload, key)
                .and_then(Value::as_f64)
                .is_some_and(|x| range.contains(x)),
            Condition::Exists { key } => lookup(payload, key).is_some(),
            Condition::Nested(inner) => inner.matches(payload),
        }
    }
}

/// A payload filter: `must` (AND), `should` (OR when non-empty), `must_not`
/// (none may hold). An empty filter matches everything (SPEC-006 §2, FLT-010).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    /// Every condition must hold.
    pub must: Vec<Condition>,
    /// At least one must hold when this clause is non-empty.
    pub should: Vec<Condition>,
    /// None may hold.
    pub must_not: Vec<Condition>,
}

impl Filter {
    /// An empty filter (matches everything).
    #[must_use]
    pub fn new() -> Self {
        Filter::default()
    }
    /// Add a `must` (AND) condition.
    #[must_use]
    pub fn must(mut self, c: Condition) -> Self {
        self.must.push(c);
        self
    }
    /// Add a `should` (OR) condition.
    #[must_use]
    pub fn should(mut self, c: Condition) -> Self {
        self.should.push(c);
        self
    }
    /// Add a `must_not` condition.
    #[must_use]
    pub fn must_not(mut self, c: Condition) -> Self {
        self.must_not.push(c);
        self
    }

    /// Whether `payload` satisfies this filter (FLT-010). `None` payload is
    /// treated as an empty object: only an empty `must`/`should` can match it.
    #[must_use]
    pub fn matches(&self, payload: Option<&Value>) -> bool {
        let all_must = self.must.iter().all(|c| c.matches(payload));
        let any_should = self.should.is_empty() || self.should.iter().any(|c| c.matches(payload));
        let no_must_not = !self.must_not.iter().any(|c| c.matches(payload));
        all_must && any_should && no_must_not
    }

    /// Reject unsupported features before evaluation (FLT-012): nested-path
    /// keys (containing `.`). Geo conditions can only enter via `from_json`,
    /// which rejects them at parse time.
    pub(crate) fn validate(&self) -> Result<()> {
        for c in self.must.iter().chain(&self.should).chain(&self.must_not) {
            match c {
                Condition::Nested(inner) => inner.validate()?,
                other => {
                    if let Some(k) = other.key() {
                        if k.contains('.') {
                            return Err(unsupported(&format!("nested-path key '{k}'")));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Parse a portable Qdrant-style filter document (SPEC-013). Unknown clause
    /// names, geo conditions, and nested-path keys are rejected (FLT-012).
    pub fn from_json(doc: &Value) -> Result<Filter> {
        let obj = doc
            .as_object()
            .ok_or_else(|| VecLiteError::InvalidArgument("filter must be a JSON object".into()))?;
        let mut filter = Filter::new();
        for (clause, conds) in obj {
            let list = conds.as_array().ok_or_else(|| {
                VecLiteError::InvalidArgument(format!("filter clause '{clause}' must be an array"))
            })?;
            let parsed: Result<Vec<Condition>> = list.iter().map(Condition::from_json).collect();
            let parsed = parsed?;
            match clause.as_str() {
                "must" => filter.must = parsed,
                "should" => filter.should = parsed,
                "must_not" => filter.must_not = parsed,
                other => {
                    return Err(VecLiteError::InvalidArgument(format!(
                        "unknown filter clause '{other}' (expected must/should/must_not)"
                    )));
                }
            }
        }
        filter.validate()?;
        Ok(filter)
    }
}

impl Condition {
    /// Parse one Qdrant-style condition object. Geo conditions and nested-path
    /// keys are rejected with `InvalidArgument` (FLT-012).
    fn from_json(doc: &Value) -> Result<Condition> {
        let obj = doc.as_object().ok_or_else(|| {
            VecLiteError::InvalidArgument("condition must be a JSON object".into())
        })?;

        // Nested composition: a condition that is itself a filter clause set.
        if obj.contains_key("must") || obj.contains_key("should") || obj.contains_key("must_not") {
            return Ok(Condition::Nested(Box::new(Filter::from_json(doc)?)));
        }

        // Explicit geo rejection (named, never ignored).
        for geo in ["geo_radius", "geo_bounding_box", "geo_polygon"] {
            if obj.contains_key(geo) {
                return Err(unsupported(&format!("geo condition '{geo}'")));
            }
        }

        let key = obj
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| VecLiteError::InvalidArgument("condition missing 'key'".into()))?;
        if key.contains('.') {
            return Err(unsupported(&format!("nested-path key '{key}'")));
        }

        if let Some(m) = obj.get("match") {
            let mobj = m
                .as_object()
                .ok_or_else(|| VecLiteError::InvalidArgument("'match' must be an object".into()))?;
            if let Some(v) = mobj.get("value") {
                return Ok(Condition::Eq {
                    key: key.to_owned(),
                    value: MatchValue::from_json(v)?,
                });
            }
            if let Some(any) = mobj.get("any") {
                let arr = any.as_array().ok_or_else(|| {
                    VecLiteError::InvalidArgument("'match.any' must be an array".into())
                })?;
                let values: Result<Vec<MatchValue>> =
                    arr.iter().map(MatchValue::from_json).collect();
                return Ok(Condition::In {
                    key: key.to_owned(),
                    values: values?,
                });
            }
            return Err(VecLiteError::InvalidArgument(
                "'match' must have 'value' or 'any'".into(),
            ));
        }

        if let Some(r) = obj.get("range") {
            let robj = r
                .as_object()
                .ok_or_else(|| VecLiteError::InvalidArgument("'range' must be an object".into()))?;
            let bound = |name: &str| -> Result<Option<f64>> {
                match robj.get(name) {
                    None | Some(Value::Null) => Ok(None),
                    Some(v) => v.as_f64().map(Some).ok_or_else(|| {
                        VecLiteError::InvalidArgument(format!("range '{name}' must be a number"))
                    }),
                }
            };
            return Ok(Condition::Range {
                key: key.to_owned(),
                range: Range {
                    gt: bound("gt")?,
                    gte: bound("gte")?,
                    lt: bound("lt")?,
                    lte: bound("lte")?,
                },
            });
        }

        if obj.get("exists").is_some() || obj.get("is_empty").is_some() {
            // `{key, exists: true}` — presence test.
            return Ok(Condition::Exists {
                key: key.to_owned(),
            });
        }

        Err(VecLiteError::InvalidArgument(format!(
            "condition on '{key}' has no match/range/exists clause"
        )))
    }
}

/// Look up a **top-level** key in a payload object.
fn lookup<'a>(payload: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    payload.and_then(Value::as_object).and_then(|o| o.get(key))
}

fn unsupported(feature: &str) -> VecLiteError {
    VecLiteError::InvalidArgument(format!("unsupported filter feature: {feature}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(v: Value) -> Value {
        v
    }

    #[test]
    fn eq_string_int_bool_semantics() {
        let p = payload(json!({"lang": "en", "year": 2024, "live": true}));
        assert!(Condition::eq("lang", "en").matches(Some(&p)));
        assert!(!Condition::eq("lang", "pt").matches(Some(&p)));
        assert!(Condition::eq("year", 2024i64).matches(Some(&p)));
        assert!(Condition::eq("live", true).matches(Some(&p)));
        assert!(!Condition::eq("live", false).matches(Some(&p)));
        // missing key never matches
        assert!(!Condition::eq("missing", "x").matches(Some(&p)));
    }

    #[test]
    fn integer_matches_float_representation() {
        let p = json!({"year": 2024.0});
        assert!(Condition::eq("year", 2024i64).matches(Some(&p)));
    }

    #[test]
    fn range_only_numeric_and_bounds() {
        let p = json!({"year": 2024, "name": "x"});
        assert!(Condition::range("year", Range::new().gte(2021.0)).matches(Some(&p)));
        assert!(!Condition::range("year", Range::new().gt(2024.0)).matches(Some(&p)));
        assert!(Condition::range("year", Range::new().gte(2000.0).lte(2024.0)).matches(Some(&p)));
        // non-numeric stored value never matches (never an error)
        assert!(!Condition::range("name", Range::new().gte(0.0)).matches(Some(&p)));
        // missing key
        assert!(!Condition::range("gone", Range::new().gte(0.0)).matches(Some(&p)));
    }

    #[test]
    fn exists_matches_null_and_presence() {
        let p = json!({"a": null, "b": 1});
        assert!(Condition::exists("a").matches(Some(&p))); // null present
        assert!(Condition::exists("b").matches(Some(&p)));
        assert!(!Condition::exists("c").matches(Some(&p)));
    }

    #[test]
    fn in_matches_any() {
        let p = json!({"lang": "pt"});
        assert!(Condition::in_("lang", vec!["en", "pt", "de"]).matches(Some(&p)));
        assert!(!Condition::in_("lang", vec!["en", "de"]).matches(Some(&p)));
    }

    #[test]
    fn must_should_must_not_combination() {
        let p = json!({"lang": "en", "year": 2024});
        // must lang=en AND year>=2021
        let f = Filter::new()
            .must(Condition::eq("lang", "en"))
            .must(Condition::range("year", Range::new().gte(2021.0)));
        assert!(f.matches(Some(&p)));

        let older = json!({"lang": "en", "year": 2020});
        assert!(!f.matches(Some(&older)));

        // should (OR): lang en or de
        let f = Filter::new()
            .should(Condition::eq("lang", "en"))
            .should(Condition::eq("lang", "de"));
        assert!(f.matches(Some(&p)));
        assert!(!f.matches(Some(&json!({"lang": "pt"}))));

        // must_not
        let f = Filter::new().must_not(Condition::eq("lang", "pt"));
        assert!(f.matches(Some(&p)));
        assert!(!f.matches(Some(&json!({"lang": "pt"}))));
    }

    #[test]
    fn empty_filter_matches_everything_including_no_payload() {
        let f = Filter::new();
        assert!(f.matches(Some(&json!({"a": 1}))));
        assert!(f.matches(None));
    }

    #[test]
    fn no_payload_fails_key_conditions() {
        let f = Filter::new().must(Condition::eq("lang", "en"));
        assert!(!f.matches(None));
    }

    #[test]
    fn nested_composition() {
        let p = json!({"lang": "en", "year": 2024});
        // must lang=en AND (year<2000 OR year>=2021)
        let inner = Filter::new()
            .should(Condition::range("year", Range::new().lt(2000.0)))
            .should(Condition::range("year", Range::new().gte(2021.0)));
        let f = Filter::new()
            .must(Condition::eq("lang", "en"))
            .must(Condition::nested(inner));
        assert!(f.matches(Some(&p)));
    }

    #[test]
    fn from_json_round_trip_semantics() {
        let doc = json!({
            "must": [
                {"key": "lang", "match": {"value": "en"}},
                {"key": "year", "range": {"gte": 2021}}
            ],
            "must_not": [
                {"key": "draft", "match": {"value": true}}
            ]
        });
        let f = Filter::from_json(&doc).unwrap_or_else(|e| panic!("{e}"));
        assert!(f.matches(Some(&json!({"lang": "en", "year": 2024}))));
        assert!(!f.matches(Some(&json!({"lang": "en", "year": 2024, "draft": true}))));
        assert!(!f.matches(Some(&json!({"lang": "pt", "year": 2024}))));
    }

    #[test]
    fn from_json_in_condition() {
        let doc = json!({"must": [{"key": "lang", "match": {"any": ["en", "de"]}}]});
        let f = Filter::from_json(&doc).unwrap_or_else(|e| panic!("{e}"));
        assert!(f.matches(Some(&json!({"lang": "de"}))));
        assert!(!f.matches(Some(&json!({"lang": "pt"}))));
    }

    #[test]
    fn geo_and_nested_path_rejected() {
        let geo = json!({"must": [{"key": "loc", "geo_radius": {"center": [0,0], "radius": 10}}]});
        assert!(matches!(
            Filter::from_json(&geo),
            Err(VecLiteError::InvalidArgument(_))
        ));
        let nested_path = json!({"must": [{"key": "a.b", "match": {"value": "x"}}]});
        assert!(matches!(
            Filter::from_json(&nested_path),
            Err(VecLiteError::InvalidArgument(_))
        ));
        // builder path: validate() catches nested-path keys too
        let f = Filter::new().must(Condition::eq("a.b", "x"));
        assert!(matches!(
            f.validate(),
            Err(VecLiteError::InvalidArgument(_))
        ));
    }

    #[test]
    fn unknown_clause_rejected() {
        let doc = json!({"maybe": [{"key": "lang", "match": {"value": "en"}}]});
        assert!(matches!(
            Filter::from_json(&doc),
            Err(VecLiteError::InvalidArgument(_))
        ));
    }

    #[test]
    fn match_value_from_impls_and_json_forms() {
        assert_eq!(MatchValue::from("en"), MatchValue::Keyword("en".into()));
        assert_eq!(
            MatchValue::from(String::from("pt")),
            MatchValue::Keyword("pt".into())
        );
        assert_eq!(MatchValue::from(42i64), MatchValue::Integer(42));

        // JSON forms: string, bool, integer ok; float and array rejected.
        let f = Filter::from_json(&json!({"must": [
            {"key": "a", "match": {"value": "s"}},
            {"key": "b", "match": {"value": true}},
            {"key": "c", "match": {"value": 7}},
        ]}))
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(f.must.len(), 3);
        assert!(
            Filter::from_json(&json!({"must": [{"key": "x", "match": {"value": 1.5}}]})).is_err()
        );
        assert!(
            Filter::from_json(&json!({"must": [{"key": "x", "match": {"value": []}}]})).is_err()
        );
    }

    #[test]
    fn match_any_parses_and_rejects_non_array() {
        let f = Filter::from_json(&json!({"must": [
            {"key": "lang", "match": {"any": ["en", "pt"]}}
        ]}))
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(f.matches(Some(&json!({"lang": "pt"}))));
        assert!(!f.matches(Some(&json!({"lang": "de"}))));

        assert!(
            Filter::from_json(&json!({"must": [
                {"key": "lang", "match": {"any": "en"}}
            ]}))
            .is_err()
        );
        assert!(
            Filter::from_json(&json!({"must": [
                {"key": "lang", "match": {}}
            ]}))
            .is_err()
        );
        assert!(
            Filter::from_json(&json!({"must": [
                {"key": "lang", "match": "en"}
            ]}))
            .is_err()
        );
    }

    #[test]
    fn range_bounds_parse_null_and_reject_non_numeric() {
        let f = Filter::from_json(&json!({"must": [
            {"key": "y", "range": {"gte": 2000, "lt": 2020, "gt": null}}
        ]}))
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(f.matches(Some(&json!({"y": 2000}))));
        assert!(f.matches(Some(&json!({"y": 2019}))));
        assert!(!f.matches(Some(&json!({"y": 2020}))));
        assert!(!f.matches(Some(&json!({"y": 1999}))));

        assert!(
            Filter::from_json(&json!({"must": [
                {"key": "y", "range": {"gte": "x"}}
            ]}))
            .is_err()
        );
        assert!(
            Filter::from_json(&json!({"must": [
                {"key": "y", "range": []}
            ]}))
            .is_err()
        );
    }

    #[test]
    fn condition_shape_errors_and_clause_array_requirement() {
        // Condition with a key but no recognized clause.
        assert!(Filter::from_json(&json!({"must": [{"key": "k"}]})).is_err());
        // Condition must be an object; clause must be an array; doc an object.
        assert!(Filter::from_json(&json!({"must": ["nope"]})).is_err());
        assert!(Filter::from_json(&json!({"must": {"key": "k"}})).is_err());
        assert!(Filter::from_json(&json!("nope")).is_err());
    }
}
