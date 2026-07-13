//! Vendored from hivellm/vectorizer vectorizer-core@3.5.0
//! (crates/vectorizer-core/src/quantization/mod.rs), Apache-2.0.
//! Adapted: trimmed to the encodings VecLite uses; crate paths localized.
//!
//! Quantization module for memory optimization
//!
//! This module implements various quantization methods to reduce memory usage
//! while maintaining search quality. Based on benchmark results showing
//! 4x memory compression with improved quality using Scalar Quantization (SQ-8bit).

// Internal data-layout file: public fields are self-documenting; the
// blanket allow keeps `cargo doc -W missing-docs` clean without padding
// every field with a tautological `///` comment. See
// phase4_enforce-public-api-docs.
#![allow(missing_docs)]

pub mod binary;
// Product Quantization is gated behind the `pq` feature: it is not yet
// wired into the public `QuantizationType`/engine surface, but is kept
// byte-identical to the vendor source for future interop (see
// crates/veclite/Cargo.toml `pq` feature doc comment).
#[cfg(feature = "pq")]
pub mod product;
pub mod scalar;
pub mod traits;

use std::fmt;

use serde::{Deserialize, Serialize};

/// Enumeration of supported quantization methods
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum QuantizationType {
    /// Scalar Quantization - 8-bit, 4-bit, 2-bit
    Scalar(u8),
    /// Product Quantization
    Product,
    /// Binary Quantization (1-bit)
    Binary,
    /// No quantization (baseline)
    None,
}

impl fmt::Display for QuantizationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuantizationType::Scalar(bits) => write!(f, "Scalar-{}bit", bits),
            QuantizationType::Product => write!(f, "Product"),
            QuantizationType::Binary => write!(f, "Binary"),
            QuantizationType::None => write!(f, "None"),
        }
    }
}

/// Error types for quantization operations
#[derive(Debug, thiserror::Error)]
pub enum QuantizationError {
    #[error("Invalid quantization parameters: {0}")]
    InvalidParameters(String),

    #[error("Quality threshold not met: {actual:.3} < {threshold:.3}")]
    QualityThresholdNotMet { actual: f32, threshold: f32 },

    #[error("Memory allocation failed: {0}")]
    MemoryAllocationFailed(String),

    #[error("Serialization failed: {0}")]
    SerializationFailed(String),

    #[error("Deserialization failed: {0}")]
    DeserializationFailed(String),

    #[error("Quantization method not supported: {0}")]
    MethodNotSupported(String),

    #[error("Vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error("Internal quantization error: {0}")]
    Internal(String),
}

/// Result type for quantization operations
pub type QuantizationResult<T> = Result<T, QuantizationError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantization_type_display() {
        assert_eq!(format!("{}", QuantizationType::Scalar(8)), "Scalar-8bit");
        assert_eq!(format!("{}", QuantizationType::Product), "Product");
        assert_eq!(format!("{}", QuantizationType::Binary), "Binary");
        assert_eq!(format!("{}", QuantizationType::None), "None");
    }
}

// Re-export main types
pub use binary::BinaryQuantization;
#[cfg(feature = "pq")]
pub use product::ProductQuantization;
pub use scalar::ScalarQuantization;
pub use traits::{QuantizationMethod, QuantizationParams, QuantizedSearch, QuantizedVectors};
