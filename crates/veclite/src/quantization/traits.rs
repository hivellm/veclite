//! Vendored from hivellm/vectorizer vectorizer-core@3.5.0
//! (crates/vectorizer-core/src/quantization/traits.rs), Apache-2.0.
//! Adapted: trimmed to the encodings VecLite uses; crate paths localized.
//!
//! Core traits for quantization methods

// Internal data-layout file: public fields are self-documenting; the
// blanket allow keeps `cargo doc -W missing-docs` clean without padding
// every field with a tautological `///` comment. See
// phase4_enforce-public-api-docs.
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

use crate::quantization::QuantizationResult;

/// Represents quantized vector data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantizedVectors {
    /// The quantized data (compressed format)
    pub data: Vec<u8>,
    /// Vector dimensions
    pub dimension: usize,
    /// Number of vectors
    pub count: usize,
    /// Quantization parameters
    pub parameters: QuantizationParams,
}

/// Parameters specific to each quantization method
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QuantizationParams {
    /// Scalar quantization parameters
    Scalar {
        bits: u8,
        min_value: f32,
        max_value: f32,
        scale: f32,
    },
    /// Product quantization parameters
    Product {
        subvector_count: usize,
        subvector_size: usize,
        codebook_size: usize,
        codebooks: Vec<Vec<Vec<f32>>>,
    },
    /// Binary quantization parameters
    Binary { threshold: f32 },
}

/// Core trait for all quantization methods
pub trait QuantizationMethod: Send + Sync {
    /// Quantize a batch of vectors
    fn quantize(&self, vectors: &[Vec<f32>]) -> QuantizationResult<QuantizedVectors>;

    /// Dequantize vectors back to float32
    fn dequantize(&self, quantized: &QuantizedVectors) -> QuantizationResult<Vec<Vec<f32>>>;

    /// Calculate memory usage for given vector count and dimension
    fn memory_usage(&self, vector_count: usize, dimension: usize) -> usize;

    /// Estimate quality loss (0.0 = no loss, 1.0 = complete loss)
    fn quality_loss(&self) -> f32;

    /// Get quantization method type
    fn method_type(&self) -> crate::quantization::QuantizationType;

    /// Validate quantization parameters
    fn validate_parameters(&self) -> QuantizationResult<()>;

    /// Serialize quantization parameters
    fn serialize_params(&self) -> QuantizationResult<QuantizationParams>;

    /// Deserialize quantization parameters
    fn deserialize_params(&mut self, params: QuantizationParams) -> QuantizationResult<()>;
}

/// Trait for quantization methods that support similarity search
pub trait QuantizedSearch {
    /// Calculate similarity between query and quantized vector
    fn similarity(&self, query: &[f32], quantized_vector: &[u8]) -> QuantizationResult<f32>;

    /// Calculate similarity between two quantized vectors
    fn quantized_similarity(
        &self,
        quantized_a: &[u8],
        quantized_b: &[u8],
    ) -> QuantizationResult<f32>;

    /// Batch similarity calculation for multiple vectors
    fn batch_similarity(
        &self,
        query: &[f32],
        quantized_vectors: &[&[u8]],
    ) -> QuantizationResult<Vec<f32>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantized_vectors_serialization() {
        let quantized = QuantizedVectors {
            data: vec![1, 2, 3, 4],
            dimension: 2,
            count: 2,
            parameters: QuantizationParams::Scalar {
                bits: 8,
                min_value: 0.0,
                max_value: 1.0,
                scale: 1.0 / 255.0,
            },
        };

        let serialized = serde_json::to_string(&quantized).unwrap_or_else(|e| panic!("{e}"));
        let deserialized: QuantizedVectors =
            serde_json::from_str(&serialized).unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(quantized.dimension, deserialized.dimension);
        assert_eq!(quantized.count, deserialized.count);
        assert_eq!(quantized.data, deserialized.data);
    }
}
