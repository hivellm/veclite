//! Vendored from hivellm/vectorizer vectorizer-core@3.5.0
//! (crates/vectorizer-core/src/quantization/scalar.rs), Apache-2.0.
//! Adapted: trimmed to the encodings VecLite uses; crate paths localized.
//!
//! Scalar Quantization implementation
//!
//! Implements scalar quantization with configurable bit depths (8-bit, 4-bit, 2-bit).
//! Based on benchmark results showing 4x memory compression with improved quality.

// Internal data-layout file: public fields are self-documenting; the
// blanket allow keeps `cargo doc -W missing-docs` clean without padding
// every field with a tautological `///` comment. See
// phase4_enforce-public-api-docs.
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

use crate::quantization::traits::*;
use crate::quantization::{QuantizationError, QuantizationResult, QuantizationType};

/// Scalar Quantization implementation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalarQuantization {
    /// Number of bits per dimension (8, 4, 2, or 1)
    pub bits: u8,
    /// Minimum value in the dataset
    pub min_value: f32,
    /// Maximum value in the dataset
    pub max_value: f32,
    /// Scaling factor for quantization
    pub scale: f32,
    /// Offset for quantization
    pub offset: f32,
    /// Number of possible quantized values
    pub quantized_levels: usize,
}

impl ScalarQuantization {
    /// Create a new scalar quantization instance
    pub fn new(bits: u8) -> QuantizationResult<Self> {
        if !matches!(bits, 1 | 2 | 4 | 8) {
            return Err(QuantizationError::InvalidParameters(format!(
                "Invalid bit depth: {}. Must be 1, 2, 4, or 8",
                bits
            )));
        }

        Ok(Self {
            bits,
            min_value: 0.0,
            max_value: 0.0,
            scale: 1.0,
            offset: 0.0,
            quantized_levels: 2usize.pow(bits as u32),
        })
    }

    /// Fit quantization parameters to a dataset
    pub fn fit(&mut self, vectors: &[Vec<f32>]) -> QuantizationResult<()> {
        if vectors.is_empty() {
            return Err(QuantizationError::InvalidParameters(
                "Cannot fit quantization to empty dataset".to_string(),
            ));
        }

        // Calculate min/max across all vectors and dimensions
        let mut min_val = f32::INFINITY;
        let mut max_val = f32::NEG_INFINITY;

        for vector in vectors {
            for &value in vector {
                min_val = min_val.min(value);
                max_val = max_val.max(value);
            }
        }

        self.min_value = min_val;
        self.max_value = max_val;
        self.scale = (max_val - min_val) / (self.quantized_levels - 1) as f32;
        self.offset = min_val;

        Ok(())
    }

    /// Quantize a single vector
    pub fn quantize_vector(&self, vector: &[f32]) -> QuantizationResult<Vec<u8>> {
        match self.bits {
            8 => self.quantize_8bit(vector),
            4 => self.quantize_4bit(vector),
            2 => self.quantize_2bit(vector),
            1 => self.quantize_1bit(vector),
            _ => Err(QuantizationError::InvalidParameters(format!(
                "Unsupported bit depth: {}",
                self.bits
            ))),
        }
    }

    /// Dequantize a single vector
    pub fn dequantize_vector(&self, quantized: &[u8]) -> QuantizationResult<Vec<f32>> {
        match self.bits {
            8 => self.dequantize_8bit(quantized),
            4 => self.dequantize_4bit(quantized),
            2 => self.dequantize_2bit(quantized),
            1 => self.dequantize_1bit(quantized),
            _ => Err(QuantizationError::InvalidParameters(format!(
                "Unsupported bit depth: {}",
                self.bits
            ))),
        }
    }

    /// 8-bit quantization (primary method from benchmarks).
    ///
    /// Routes through `crate::simd::quantize_f32_to_u8` so the
    /// dispatched backend handles the subtract-scale-clamp-round-cast
    /// chain on vector lanes. The SIMD primitive handles the
    /// constant-valued dataset case (`scale == 0.0`) internally by
    /// writing all-zero codes — matching the pre-7f scalar loop's
    /// silent-divide-by-zero-then-clamp semantics without the panic
    /// risk.
    fn quantize_8bit(&self, vector: &[f32]) -> QuantizationResult<Vec<u8>> {
        let mut quantized = vec![0u8; vector.len()];
        crate::simd::quantize_f32_to_u8(
            vector,
            &mut quantized,
            self.scale,
            self.offset,
            self.quantized_levels as u32,
        );
        Ok(quantized)
    }

    /// 8-bit dequantization.
    ///
    /// Routes through `crate::simd::dequantize_u8_to_f32` so the
    /// dispatched backend handles the cast-multiply-add chain on
    /// vector lanes.
    fn dequantize_8bit(&self, quantized: &[u8]) -> QuantizationResult<Vec<f32>> {
        let mut dequantized = vec![0.0f32; quantized.len()];
        crate::simd::dequantize_u8_to_f32(quantized, &mut dequantized, self.scale, self.offset);
        Ok(dequantized)
    }

    /// 4-bit quantization (packed)
    fn quantize_4bit(&self, vector: &[f32]) -> QuantizationResult<Vec<u8>> {
        let mut quantized = Vec::with_capacity((vector.len() + 1) / 2);

        for chunk in vector.chunks(2) {
            let mut packed = 0u8;

            for (i, &value) in chunk.iter().enumerate() {
                let normalized = (value - self.offset) / self.scale;
                let clamped = normalized.clamp(0.0, (self.quantized_levels - 1) as f32);
                let quantized_value = clamped.round() as u8;

                if i == 0 {
                    packed |= quantized_value;
                } else {
                    packed |= (quantized_value << 4);
                }
            }

            quantized.push(packed);
        }

        Ok(quantized)
    }

    /// 4-bit dequantization (unpacked)
    fn dequantize_4bit(&self, quantized: &[u8]) -> QuantizationResult<Vec<f32>> {
        let mut dequantized = Vec::with_capacity(quantized.len() * 2);

        for &packed in quantized {
            // Extract lower 4 bits
            let lower = packed & 0x0F;
            let value1 = self.offset + (lower as f32) * self.scale;
            dequantized.push(value1);

            // Extract upper 4 bits
            let upper = (packed & 0xF0) >> 4;
            let value2 = self.offset + (upper as f32) * self.scale;
            dequantized.push(value2);
        }

        Ok(dequantized)
    }

    /// 2-bit quantization (packed)
    fn quantize_2bit(&self, vector: &[f32]) -> QuantizationResult<Vec<u8>> {
        let mut quantized = Vec::with_capacity((vector.len() + 3) / 4);

        for chunk in vector.chunks(4) {
            let mut packed = 0u8;

            for (i, &value) in chunk.iter().enumerate() {
                let normalized = (value - self.offset) / self.scale;
                let clamped = normalized.clamp(0.0, (self.quantized_levels - 1) as f32);
                let quantized_value = clamped.round() as u8;

                packed |= (quantized_value << (i * 2));
            }

            quantized.push(packed);
        }

        Ok(quantized)
    }

    /// 2-bit dequantization (unpacked)
    fn dequantize_2bit(&self, quantized: &[u8]) -> QuantizationResult<Vec<f32>> {
        let mut dequantized = Vec::with_capacity(quantized.len() * 4);

        for &packed in quantized {
            for i in 0..4 {
                let value_bits = (packed >> (i * 2)) & 0x03;
                let value = self.offset + (value_bits as f32) * self.scale;
                dequantized.push(value);
            }
        }

        Ok(dequantized)
    }

    /// 1-bit quantization (binary)
    fn quantize_1bit(&self, vector: &[f32]) -> QuantizationResult<Vec<u8>> {
        let threshold = (self.min_value + self.max_value) / 2.0;
        let mut quantized = Vec::with_capacity((vector.len() + 7) / 8);

        for chunk in vector.chunks(8) {
            let mut packed = 0u8;

            for (i, &value) in chunk.iter().enumerate() {
                if value >= threshold {
                    packed |= 1 << i;
                }
            }

            quantized.push(packed);
        }

        Ok(quantized)
    }

    /// 1-bit dequantization (binary)
    fn dequantize_1bit(&self, quantized: &[u8]) -> QuantizationResult<Vec<f32>> {
        let threshold = (self.min_value + self.max_value) / 2.0;
        let mut dequantized = Vec::with_capacity(quantized.len() * 8);

        for &packed in quantized {
            for i in 0..8 {
                let bit = (packed >> i) & 1;
                let value = if bit == 1 {
                    self.max_value
                } else {
                    self.min_value
                };
                dequantized.push(value);
            }
        }

        Ok(dequantized)
    }

    /// Calculate quantization error for quality assessment
    pub fn calculate_quantization_error(
        &self,
        original: &[f32],
        quantized: &[u8],
    ) -> QuantizationResult<f32> {
        let dequantized = self.dequantize_vector(quantized)?;

        if original.len() != dequantized.len() {
            return Err(QuantizationError::DimensionMismatch {
                expected: original.len(),
                actual: dequantized.len(),
            });
        }

        let mse = original
            .iter()
            .zip(dequantized.iter())
            .map(|(orig, deq)| (orig - deq).powi(2))
            .sum::<f32>()
            / original.len() as f32;

        Ok(mse)
    }

    /// Calculate theoretical compression ratio
    pub fn theoretical_compression_ratio(&self) -> f32 {
        let original_bits = 32.0; // f32
        let quantized_bits = self.bits as f32;
        original_bits / quantized_bits
    }
}

impl QuantizationMethod for ScalarQuantization {
    fn quantize(&self, vectors: &[Vec<f32>]) -> QuantizationResult<QuantizedVectors> {
        if vectors.is_empty() {
            return Err(QuantizationError::InvalidParameters(
                "Cannot quantize empty vector set".to_string(),
            ));
        }

        let dimension = vectors[0].len();
        let mut all_quantized = Vec::new();

        for vector in vectors {
            if vector.len() != dimension {
                return Err(QuantizationError::DimensionMismatch {
                    expected: dimension,
                    actual: vector.len(),
                });
            }

            let quantized = self.quantize_vector(vector)?;
            all_quantized.extend(quantized);
        }

        let parameters = self.serialize_params()?;

        Ok(QuantizedVectors {
            data: all_quantized,
            dimension,
            count: vectors.len(),
            parameters,
        })
    }

    fn dequantize(&self, quantized: &QuantizedVectors) -> QuantizationResult<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(quantized.count);
        let bytes_per_vector = match self.bits {
            8 => quantized.dimension,
            4 => (quantized.dimension + 1) / 2,
            2 => (quantized.dimension + 3) / 4,
            1 => (quantized.dimension + 7) / 8,
            _ => {
                return Err(QuantizationError::InvalidParameters(format!(
                    "Unsupported bit depth: {}",
                    self.bits
                )));
            }
        };

        for i in 0..quantized.count {
            let start = i * bytes_per_vector;
            let end = start + bytes_per_vector;

            if end > quantized.data.len() {
                return Err(QuantizationError::InvalidParameters(
                    "Quantized data length mismatch".to_string(),
                ));
            }

            let vector_data = &quantized.data[start..end];
            let dequantized_vector = self.dequantize_vector(vector_data)?;

            // Truncate to original dimension (for packed formats)
            let vector = dequantized_vector[..quantized.dimension].to_vec();
            vectors.push(vector);
        }

        Ok(vectors)
    }

    fn memory_usage(&self, vector_count: usize, dimension: usize) -> usize {
        match self.bits {
            8 => vector_count * dimension,
            4 => vector_count * (dimension + 1) / 2,
            2 => vector_count * (dimension + 3) / 4,
            1 => vector_count * (dimension + 7) / 8,
            _ => 0,
        }
    }

    fn quality_loss(&self) -> f32 {
        // Theoretical quality loss based on quantization error
        let quantization_step = self.scale;
        let signal_range = self.max_value - self.min_value;

        if signal_range == 0.0 {
            return 0.0;
        }

        // Quality loss is proportional to quantization step size
        quantization_step / signal_range
    }

    fn method_type(&self) -> QuantizationType {
        QuantizationType::Scalar(self.bits)
    }

    fn validate_parameters(&self) -> QuantizationResult<()> {
        if !matches!(self.bits, 1 | 2 | 4 | 8) {
            return Err(QuantizationError::InvalidParameters(format!(
                "Invalid bit depth: {}",
                self.bits
            )));
        }

        if self.min_value >= self.max_value {
            return Err(QuantizationError::InvalidParameters(
                "min_value must be less than max_value".to_string(),
            ));
        }

        if self.scale <= 0.0 {
            return Err(QuantizationError::InvalidParameters(
                "scale must be positive".to_string(),
            ));
        }

        Ok(())
    }

    fn serialize_params(&self) -> QuantizationResult<QuantizationParams> {
        Ok(QuantizationParams::Scalar {
            bits: self.bits,
            min_value: self.min_value,
            max_value: self.max_value,
            scale: self.scale,
        })
    }

    fn deserialize_params(&mut self, params: QuantizationParams) -> QuantizationResult<()> {
        if let QuantizationParams::Scalar {
            bits,
            min_value,
            max_value,
            scale,
        } = params
        {
            self.bits = bits;
            self.min_value = min_value;
            self.max_value = max_value;
            self.scale = scale;
            // BUGFIX (VecLite-only, not upstream): the vendor never restored
            // `offset` here, leaving it at the struct default (0.0) after a
            // serialize -> deserialize round trip and silently corrupting
            // dequantization for any dataset with `min_value != 0.0`.
            // `offset` is always set equal to `min_value` in `fit` (see
            // above), so restore that same invariant here. This does NOT
            // change the `QuantizationParams::Scalar` wire format — `offset`
            // is still derived, not serialized (see `serialize_params`).
            self.offset = min_value;
            self.quantized_levels = 2usize.pow(bits as u32);
            Ok(())
        } else {
            Err(QuantizationError::InvalidParameters(
                "Parameter type mismatch for ScalarQuantization".to_string(),
            ))
        }
    }
}

impl QuantizedSearch for ScalarQuantization {
    fn similarity(&self, query: &[f32], quantized_vector: &[u8]) -> QuantizationResult<f32> {
        let dequantized = self.dequantize_vector(quantized_vector)?;

        if query.len() != dequantized.len() {
            return Err(QuantizationError::DimensionMismatch {
                expected: query.len(),
                actual: dequantized.len(),
            });
        }

        // Calculate cosine similarity
        let dot_product: f32 = query
            .iter()
            .zip(dequantized.iter())
            .map(|(a, b)| a * b)
            .sum();

        let query_norm: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        let vector_norm: f32 = dequantized.iter().map(|x| x * x).sum::<f32>().sqrt();

        if query_norm == 0.0 || vector_norm == 0.0 {
            return Ok(0.0);
        }

        Ok(dot_product / (query_norm * vector_norm))
    }

    fn quantized_similarity(
        &self,
        quantized_a: &[u8],
        quantized_b: &[u8],
    ) -> QuantizationResult<f32> {
        let dequantized_a = self.dequantize_vector(quantized_a)?;
        let dequantized_b = self.dequantize_vector(quantized_b)?;

        if dequantized_a.len() != dequantized_b.len() {
            return Err(QuantizationError::DimensionMismatch {
                expected: dequantized_a.len(),
                actual: dequantized_b.len(),
            });
        }

        // Calculate cosine similarity
        let dot_product: f32 = dequantized_a
            .iter()
            .zip(dequantized_b.iter())
            .map(|(a, b)| a * b)
            .sum();

        let norm_a: f32 = dequantized_a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = dequantized_b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return Ok(0.0);
        }

        Ok(dot_product / (norm_a * norm_b))
    }

    fn batch_similarity(
        &self,
        query: &[f32],
        quantized_vectors: &[&[u8]],
    ) -> QuantizationResult<Vec<f32>> {
        let mut similarities = Vec::with_capacity(quantized_vectors.len());

        for quantized_vector in quantized_vectors {
            let similarity = self.similarity(query, quantized_vector)?;
            similarities.push(similarity);
        }

        Ok(similarities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scalar_quantization_8bit() {
        let mut sq = ScalarQuantization::new(8).unwrap_or_else(|e| panic!("{e}"));

        let vectors = vec![
            vec![0.1, 0.5, 0.9],
            vec![0.2, 0.6, 0.8],
            vec![0.3, 0.7, 0.7],
        ];

        sq.fit(&vectors).unwrap_or_else(|e| panic!("{e}"));

        let quantized = sq.quantize(&vectors).unwrap_or_else(|e| panic!("{e}"));
        let dequantized = sq.dequantize(&quantized).unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(quantized.count, 3);
        assert_eq!(quantized.dimension, 3);
        assert_eq!(dequantized.len(), 3);

        // Check that dequantized values are close to original
        for (orig, deq) in vectors.iter().zip(dequantized.iter()) {
            for (o, d) in orig.iter().zip(deq.iter()) {
                assert!(
                    (o - d).abs() < 0.1,
                    "Quantization error too large: {} vs {}",
                    o,
                    d
                );
            }
        }
    }

    #[test]
    fn test_scalar_quantization_4bit() {
        let mut sq = ScalarQuantization::new(4).unwrap_or_else(|e| panic!("{e}"));

        let vectors = vec![vec![0.1, 0.5, 0.9, 0.3], vec![0.2, 0.6, 0.8, 0.4]];

        sq.fit(&vectors).unwrap_or_else(|e| panic!("{e}"));

        let quantized = sq.quantize(&vectors).unwrap_or_else(|e| panic!("{e}"));
        let dequantized = sq.dequantize(&quantized).unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(quantized.count, 2);
        assert_eq!(quantized.dimension, 4);

        // 4-bit should have more quantization error than 8-bit
        let mut total_error = 0.0;
        for (orig, deq) in vectors.iter().zip(dequantized.iter()) {
            for (o, d) in orig.iter().zip(deq.iter()) {
                total_error += (o - d).abs();
            }
        }

        assert!(total_error > 0.0, "Should have some quantization error");
    }

    #[test]
    fn test_memory_usage_calculation() {
        let sq8 = ScalarQuantization::new(8).unwrap_or_else(|e| panic!("{e}"));
        let sq4 = ScalarQuantization::new(4).unwrap_or_else(|e| panic!("{e}"));
        let sq2 = ScalarQuantization::new(2).unwrap_or_else(|e| panic!("{e}"));
        let sq1 = ScalarQuantization::new(1).unwrap_or_else(|e| panic!("{e}"));

        let vector_count = 1000;
        let dimension = 512;

        assert_eq!(sq8.memory_usage(vector_count, dimension), 1000 * 512);
        assert_eq!(
            sq4.memory_usage(vector_count, dimension),
            1000 * (512 + 1) / 2
        );
        assert_eq!(
            sq2.memory_usage(vector_count, dimension),
            1000 * (512 + 3) / 4
        );
        assert_eq!(
            sq1.memory_usage(vector_count, dimension),
            1000 * (512 + 7) / 8
        );
    }

    #[test]
    fn test_similarity_calculation() {
        let mut sq = ScalarQuantization::new(8).unwrap_or_else(|e| panic!("{e}"));

        let vectors = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ];

        sq.fit(&vectors).unwrap_or_else(|e| panic!("{e}"));

        let quantized = sq.quantize(&vectors).unwrap_or_else(|e| panic!("{e}"));
        let query = vec![1.0, 0.0, 0.0];

        // Calculate similarity with first vector (should be high)
        let first_vector_data = &quantized.data[0..quantized.dimension];
        let similarity = sq
            .similarity(&query, first_vector_data)
            .unwrap_or_else(|e| panic!("{e}"));

        assert!(
            similarity > 0.9,
            "Similarity should be high for identical vectors"
        );
    }

    #[test]
    fn test_validation() {
        let mut sq = ScalarQuantization::new(8).unwrap_or_else(|e| panic!("{e}"));

        // Test valid parameters
        sq.min_value = 0.0;
        sq.max_value = 1.0;
        sq.scale = 0.1;
        assert!(sq.validate_parameters().is_ok());

        // Test invalid bit depth
        sq.bits = 16;
        assert!(sq.validate_parameters().is_err());

        // Test invalid min/max
        sq.bits = 8;
        sq.min_value = 1.0;
        sq.max_value = 0.0;
        assert!(sq.validate_parameters().is_err());
    }

    /// Proves the `deserialize_params` offset fix: fit on a dataset with a
    /// nonzero `min_value`, round-trip through serialize/deserialize into a
    /// fresh instance, and check dequantization matches. Before the fix,
    /// `restored.offset` stayed at the struct default (0.0) instead of
    /// `min_value`, corrupting every dequantized value by `-min_value`.
    #[test]
    fn test_deserialize_params_restores_offset() {
        let mut sq = ScalarQuantization::new(8).unwrap_or_else(|e| panic!("{e}"));

        // Nonzero min_value: a lost offset (defaulting to 0.0) would
        // silently corrupt dequantization instead of being masked by
        // min_value coincidentally already being 0.0.
        let vectors = vec![vec![10.0, 20.0, 30.0], vec![15.0, 25.0, 35.0]];
        sq.fit(&vectors).unwrap_or_else(|e| panic!("{e}"));

        let params = sq.serialize_params().unwrap_or_else(|e| panic!("{e}"));

        let mut restored = ScalarQuantization::new(8).unwrap_or_else(|e| panic!("{e}"));
        restored
            .deserialize_params(params)
            .unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(restored.offset, sq.min_value);

        let quantized = sq
            .quantize_vector(&vectors[0])
            .unwrap_or_else(|e| panic!("{e}"));
        let expected = sq
            .dequantize_vector(&quantized)
            .unwrap_or_else(|e| panic!("{e}"));
        let actual = restored
            .dequantize_vector(&quantized)
            .unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(actual, expected);
    }

    /// Pins the SQ-8 encoding to a hand-computed byte sequence, per the
    /// module formula: `scale = (max - min) / 255`,
    /// `code = round((v - min) / scale)`. `min = 0.0`, `max = 510.0` keeps
    /// `scale == 2.0` binary-exact (no float rounding to second-guess), so
    /// every intermediate (`inv_scale = 0.5`, `1.5`, `2.5`, `255.0`) is also
    /// binary-exact — including the `round-half-away-from-zero` tie at
    /// `2.5`.
    #[test]
    fn test_quantize_8bit_stability_pinned_bytes() {
        let corpus = vec![vec![0.0f32, 3.0, 5.0, 510.0]];

        let mut sq = ScalarQuantization::new(8).unwrap_or_else(|e| panic!("{e}"));
        sq.fit(&corpus).unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(sq.min_value, 0.0);
        assert_eq!(sq.max_value, 510.0);
        assert_eq!(sq.scale, 2.0);

        let quantized = sq
            .quantize_vector(&corpus[0])
            .unwrap_or_else(|e| panic!("{e}"));

        // code(0.0)   = round(0.0 / 2.0) = 0
        // code(3.0)   = round(1.5)       = 2  (round-half-away-from-zero)
        // code(5.0)   = round(2.5)       = 3
        // code(510.0) = round(255.0)     = 255
        let expected: Vec<u8> = vec![0, 2, 3, 255];
        assert_eq!(quantized, expected);
    }

    fn fitted(bits: u8, vectors: &[Vec<f32>]) -> ScalarQuantization {
        let mut sq = ScalarQuantization::new(bits).unwrap_or_else(|e| panic!("{e}"));
        sq.fit(vectors).unwrap_or_else(|e| panic!("{e}"));
        sq
    }

    #[test]
    fn one_and_two_bit_round_trip_within_step() {
        let vectors = vec![
            vec![0.0f32, 0.25, 0.5, 0.75, 1.0],
            vec![1.0, 0.75, 0.5, 0.25, 0.0],
        ];
        for bits in [1u8, 2] {
            let sq = fitted(bits, &vectors);
            let q = sq.quantize(&vectors).unwrap_or_else(|e| panic!("{e}"));
            let d = sq.dequantize(&q).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(d.len(), 2, "bits={bits}");
            // One quantization step is the max representable error.
            let levels = f32::from((1u8 << bits) - 1);
            let step = (sq.max_value - sq.min_value) / levels;
            for (orig, deq) in vectors.iter().zip(d.iter()) {
                for (o, dq) in orig.iter().zip(deq.iter()) {
                    assert!((o - dq).abs() <= step + 1e-6, "bits={bits}: {o} vs {dq}");
                }
            }
        }
    }

    #[test]
    fn invalid_bit_depth_rejected_at_new() {
        assert!(ScalarQuantization::new(3).is_err());
        assert!(ScalarQuantization::new(16).is_err());
        assert!(ScalarQuantization::new(0).is_err());
    }

    #[test]
    fn fit_and_quantize_reject_empty_input() {
        let mut sq = ScalarQuantization::new(8).unwrap_or_else(|e| panic!("{e}"));
        assert!(sq.fit(&[]).is_err());
        sq.fit(&[vec![0.0, 1.0]]).unwrap_or_else(|e| panic!("{e}"));
        assert!(sq.quantize(&[]).is_err());
    }

    #[test]
    fn quantize_batch_rejects_dimension_mismatch() {
        let sq = fitted(8, &[vec![0.0, 1.0, 2.0]]);
        let bad = vec![vec![0.0, 1.0, 2.0], vec![0.0, 1.0]];
        assert!(matches!(
            sq.quantize(&bad),
            Err(QuantizationError::DimensionMismatch { .. })
        ));
    }

    #[test]
    fn dequantize_batch_rejects_length_mismatch() {
        let vectors = vec![vec![0.0f32, 0.5, 1.0]];
        let sq = fitted(8, &vectors);
        let mut q = sq.quantize(&vectors).unwrap_or_else(|e| panic!("{e}"));
        q.data.pop(); // corrupt: one byte short
        assert!(sq.dequantize(&q).is_err());
    }

    #[test]
    fn four_bit_odd_dimension_packs_and_unpacks() {
        // Odd dim exercises the half-byte tail in quantize_4bit/dequantize_4bit.
        let vectors = vec![vec![0.0f32, 0.5, 1.0], vec![1.0, 0.5, 0.0]];
        let sq = fitted(4, &vectors);
        let q = sq.quantize(&vectors).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(q.data.len(), 2 * 2); // ceil(3/2) bytes per vector
        let d = sq.dequantize(&q).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(d[0].len(), 3);
        let step = (sq.max_value - sq.min_value) / 15.0;
        for (o, dq) in vectors[0].iter().zip(d[0].iter()) {
            assert!((o - dq).abs() <= step + 1e-6);
        }
    }

    #[test]
    fn quantization_error_and_compression_ratio() {
        let vectors = vec![vec![0.0f32, 0.5, 1.0], vec![0.25, 0.5, 0.75]];
        let sq = fitted(8, &vectors);
        let codes = sq
            .quantize_vector(&vectors[0])
            .unwrap_or_else(|e| panic!("{e}"));
        let err = sq
            .calculate_quantization_error(&vectors[0], &codes)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!((0.0..0.05).contains(&err), "mse {err}");
        assert!((sq.theoretical_compression_ratio() - 4.0).abs() < 1e-6);
        assert!((fitted(4, &vectors).theoretical_compression_ratio() - 8.0).abs() < 1e-6);
        assert!((fitted(1, &vectors).theoretical_compression_ratio() - 32.0).abs() < 1e-6);
    }

    #[test]
    fn quality_loss_and_method_type_per_depth() {
        use crate::quantization::QuantizationType;
        let corpus = vec![vec![0.0f32, 1.0]];
        let mut prev = 0.0f32;
        for bits in [8u8, 4, 2, 1] {
            let sq = fitted(bits, &corpus);
            // Fitted loss = step/range = 1/levels: grows as the depth shrinks.
            let loss = sq.quality_loss();
            assert!(loss > prev, "bits={bits}: loss {loss} !> {prev}");
            prev = loss;
            assert_eq!(sq.method_type(), QuantizationType::Scalar(bits));
        }
        // Unfitted: zero signal range reports zero loss (the documented guard).
        let unfitted = ScalarQuantization::new(8).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(unfitted.quality_loss(), 0.0);
    }

    #[test]
    fn deserialize_params_rejects_wrong_variant() {
        let mut sq = ScalarQuantization::new(8).unwrap_or_else(|e| panic!("{e}"));
        let wrong = QuantizationParams::Binary { threshold: 0.5 };
        assert!(sq.deserialize_params(wrong).is_err());
    }

    #[test]
    fn quantized_similarity_and_batch_similarity() {
        let vectors = vec![
            vec![1.0f32, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![1.0, 0.1, 0.0],
        ];
        let sq = fitted(8, &vectors);
        let q = sq.quantize(&vectors).unwrap_or_else(|e| panic!("{e}"));
        let dim = q.dimension;
        let a = &q.data[0..dim];
        let b = &q.data[dim..2 * dim];
        let c = &q.data[2 * dim..3 * dim];
        let ab = sq
            .quantized_similarity(a, b)
            .unwrap_or_else(|e| panic!("{e}"));
        let ac = sq
            .quantized_similarity(a, c)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(ac > ab, "near-parallel beats orthogonal: {ac} vs {ab}");

        let rows: Vec<&[u8]> = vec![a, b, c];
        let scores = sq
            .batch_similarity(&[1.0, 0.0, 0.0], &rows)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(scores.len(), 3);
        assert!(scores[0] > scores[1]);
        assert!(scores[2] > scores[1]);
    }

    #[test]
    fn similarity_zero_norm_is_zero_and_mismatch_errors() {
        let vectors = vec![vec![0.0f32, 0.0, 0.0], vec![1.0, 1.0, 1.0]];
        let sq = fitted(8, &vectors);
        let q = sq.quantize(&vectors).unwrap_or_else(|e| panic!("{e}"));
        let zero = &q.data[0..q.dimension];
        let s = sq
            .similarity(&[1.0, 0.0, 0.0], zero)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s, 0.0);
        assert!(sq.similarity(&[1.0, 0.0], zero).is_err());
    }
}
