//! Data model: points, sparse vectors, search hits, and identifier
//! validation (SPEC-001 §3).

use serde::{Deserialize, Serialize};

use crate::error::{Result, VecLiteError};

/// Maximum vector id length in bytes (SPEC-002 §8 limits).
pub(crate) const MAX_ID_BYTES: usize = 512;
/// Maximum collection name length in bytes (SPEC-001 CORE-011).
pub(crate) const MAX_COLLECTION_NAME_BYTES: usize = 255;

/// A vector with its id and optional sparse lane / payload.
///
/// ```
/// use veclite::Point;
///
/// let p = Point::new("id-1", vec![0.1, 0.2, 0.3])
///     .payload(serde_json::json!({ "lang": "en" }));
/// assert_eq!(p.id, "id-1");
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// UTF-8 id, 1–512 bytes (CORE-010).
    pub id: String,
    /// Dense vector; length must equal the collection dimension (CORE-012).
    pub vector: Vec<f32>,
    /// Optional sparse lane for hybrid search (SPEC-007).
    pub sparse: Option<SparseVector>,
    /// Optional JSON payload (SPEC-006).
    pub payload: Option<serde_json::Value>,
}

impl Point {
    /// A point with no sparse lane and no payload.
    pub fn new(id: impl Into<String>, vector: Vec<f32>) -> Self {
        Point {
            id: id.into(),
            vector,
            sparse: None,
            payload: None,
        }
    }

    /// Attach a JSON payload.
    pub fn payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = Some(payload);
        self
    }

    /// Attach an explicit sparse vector (BYO sparse lane, SPEC-007 HYB-002).
    pub fn sparse(mut self, sparse: SparseVector) -> Self {
        self.sparse = Some(sparse);
        self
    }
}

/// Sparse vector: parallel `indices`/`values` arrays (SPEC-007 HYB-001).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SparseVector {
    /// Term indices, strictly increasing.
    pub indices: Vec<u32>,
    /// Weights, one per index.
    pub values: Vec<f32>,
}

impl SparseVector {
    /// Enforce the invariants (SPEC-007 HYB-001): `indices` strictly increasing,
    /// `values.len() == indices.len()`, all values finite.
    pub(crate) fn validate(&self) -> Result<()> {
        if self.indices.len() != self.values.len() {
            return Err(VecLiteError::InvalidArgument(format!(
                "sparse vector has {} indices but {} values",
                self.indices.len(),
                self.values.len()
            )));
        }
        if self.values.iter().any(|v| !v.is_finite()) {
            return Err(VecLiteError::InvalidArgument(
                "sparse vector values must be finite".into(),
            ));
        }
        if self.indices.windows(2).any(|w| w[0] >= w[1]) {
            return Err(VecLiteError::InvalidArgument(
                "sparse vector indices must be strictly increasing (sorted, unique)".into(),
            ));
        }
        Ok(())
    }

    /// Dot product over the shared term space (both operands are sorted by
    /// index, so this is a linear merge). Used for sparse scoring (HYB-003).
    pub(crate) fn dot(&self, other: &SparseVector) -> f32 {
        let (mut i, mut j) = (0usize, 0usize);
        let mut sum = 0.0f32;
        while i < self.indices.len() && j < other.indices.len() {
            match self.indices[i].cmp(&other.indices[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    sum += self.values[i] * other.values[j];
                    i += 1;
                    j += 1;
                }
            }
        }
        sum
    }
}

/// One search result (SPEC-004 §4).
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// Id of the matched point.
    pub id: String,
    /// Similarity or distance score, ordered per SPEC-001 CORE-035.
    pub score: f32,
    /// Payload, present when the query ran `with_payload` (default true).
    pub payload: Option<serde_json::Value>,
    /// Stored vector, present when the query ran `with_vector` (default false).
    pub vector: Option<Vec<f32>>,
}

/// Validate a vector id (CORE-010): 1–512 bytes of UTF-8.
pub(crate) fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(VecLiteError::InvalidArgument("id must not be empty".into()));
    }
    if id.len() > MAX_ID_BYTES {
        return Err(VecLiteError::InvalidArgument(format!(
            "id exceeds {MAX_ID_BYTES} bytes: {} bytes",
            id.len()
        )));
    }
    Ok(())
}

/// Validate a collection name (CORE-011): 1–255 bytes, no `/`, `\`, or NUL,
/// no leading/trailing whitespace.
pub(crate) fn validate_collection_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(VecLiteError::InvalidArgument(
            "collection name must not be empty".into(),
        ));
    }
    if name.len() > MAX_COLLECTION_NAME_BYTES {
        return Err(VecLiteError::InvalidArgument(format!(
            "collection name exceeds {MAX_COLLECTION_NAME_BYTES} bytes: {} bytes",
            name.len()
        )));
    }
    if name.contains(['/', '\\', '\0']) {
        return Err(VecLiteError::InvalidArgument(format!(
            "collection name contains a forbidden character (/, \\, or NUL): {name:?}"
        )));
    }
    if name.trim() != name {
        return Err(VecLiteError::InvalidArgument(format!(
            "collection name has leading or trailing whitespace: {name:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_validation_bounds() {
        assert!(validate_id("a").is_ok());
        assert!(validate_id(&"x".repeat(MAX_ID_BYTES)).is_ok());
        assert!(matches!(
            validate_id(""),
            Err(VecLiteError::InvalidArgument(_))
        ));
        assert!(matches!(
            validate_id(&"x".repeat(MAX_ID_BYTES + 1)),
            Err(VecLiteError::InvalidArgument(_))
        ));
    }

    #[test]
    fn id_length_is_measured_in_bytes() {
        // 171 four-byte code points = 684 bytes > 512, though only 171 chars.
        let id = "\u{1F600}".repeat(171);
        assert!(validate_id(&id).is_err());
    }

    #[test]
    fn collection_name_validation() {
        assert!(validate_collection_name("docs").is_ok());
        assert!(validate_collection_name(&"n".repeat(255)).is_ok());
        for bad in ["", " docs", "docs ", "a/b", "a\\b", "a\0b"] {
            assert!(
                matches!(
                    validate_collection_name(bad),
                    Err(VecLiteError::InvalidArgument(_))
                ),
                "expected rejection for {bad:?}"
            );
        }
        assert!(validate_collection_name(&"n".repeat(256)).is_err());
    }

    #[test]
    fn point_builder() {
        let p = Point::new("id-1", vec![1.0, 2.0])
            .payload(serde_json::json!({"k": 1}))
            .sparse(SparseVector {
                indices: vec![1, 5],
                values: vec![0.5, 0.25],
            });
        assert_eq!(p.id, "id-1");
        assert_eq!(p.vector, vec![1.0, 2.0]);
        assert!(p.payload.is_some());
        assert!(p.sparse.is_some());
    }
}
