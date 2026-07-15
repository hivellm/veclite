//! Vendored from hivellm/vectorizer vectorizer-core@3.5.0
//! (crates/vectorizer-core/src/simd/backend.rs), Apache-2.0. Adapted:
//! scalar-only (ISA backends deferred), crate paths localized.
//!
//! The `SimdBackend` trait — the contract every per-ISA implementation
//! satisfies.
//!
//! v1 of the trait (phase 7a) covered the four `f32` primitives the
//! hot paths exercise today: `dot_product`,
//! `euclidean_distance_squared`, `cosine_similarity`, and `l2_norm`.
//! Phase 7b adds `int8_dot_product` for the upcoming quantization
//! work (phase 7f's INT8 asymmetric distance) — backends that have a
//! single-instruction implementation (AVX-512 VNNI, NEON DOTPROD)
//! override it; everything else inherits the scalar fallback.
//!
//! Adding a method here without a matching scalar fallback breaks
//! every backend at once, which is the desired tripwire — but
//! providing the fallback keeps the per-ISA backends simple.
//!
//! Invariants every implementation MUST uphold:
//!
//! - The two slices have equal length. Caller checks; backends may
//!   `debug_assert` but should NOT panic in release. Mismatched
//!   lengths are a correctness bug at the call site.
//! - `cosine_similarity` assumes both inputs are pre-normalised; it
//!   is implemented as a clamped dot product, NOT as
//!   `dot / (|a| * |b|)`. Callers that need full cosine should
//!   normalise first or use `models::DistanceCalculator::cosine_similarity`
//!   from `src/models/mod.rs`.
//! - `euclidean_distance_squared` returns the SQUARED distance to
//!   save the `sqrt` for callers that only compare distances. The
//!   convenience function `crate::simd::euclidean_distance` does the
//!   sqrt for callers that need it.
//! - Every method MUST return the same value (within f32 rounding) as
//!   the [`crate::simd::scalar::ScalarBackend`] oracle. The
//!   `tests/simd/scalar_oracle.rs` integration test pins this on
//!   random vectors.

/// Implemented by every per-ISA backend. `Send + Sync + 'static`
/// because the dispatcher caches a `&'static dyn SimdBackend` and
/// hands it across threads.
pub trait SimdBackend: Send + Sync + 'static {
    /// Sum of pairwise products: `∑ a[i] * b[i]`.
    fn dot_product(&self, a: &[f32], b: &[f32]) -> f32;

    /// `∑ (a[i] - b[i])²`. Caller takes `sqrt` if Euclidean distance
    /// (rather than its square) is needed.
    fn euclidean_distance_squared(&self, a: &[f32], b: &[f32]) -> f32;

    /// Cosine similarity ASSUMING pre-normalised inputs — implemented
    /// as a clamped dot product. See trait-level docs.
    fn cosine_similarity(&self, a: &[f32], b: &[f32]) -> f32;

    /// L2 norm: `sqrt(∑ a[i]²)`.
    fn l2_norm(&self, a: &[f32]) -> f32;

    /// INT8 dot product: `∑ a[i] * b[i]` returning an `i32`. Used by
    /// the phase 7f quantized-distance code path. Default impl is a
    /// straight scalar loop. AVX2 (`x86::avx2`) and AVX-512 VNNI
    /// (`x86::avx512_vnni`) override this on x86_64; NEON
    /// (`aarch64::neon`) and SVE2 (`aarch64::sve2`) override it on
    /// aarch64. Plain SSE2, AVX-512F, and SVE inherit this scalar
    /// default. The `i32` accumulator absorbs the worst-case
    /// `127 * 127 * len` without overflow for `len < 130_000`.
    fn int8_dot_product(&self, a: &[i8], b: &[i8]) -> i32 {
        debug_assert_eq!(a.len(), b.len(), "Vectors must have same length");
        a.iter()
            .zip(b.iter())
            .map(|(x, y)| (*x as i32) * (*y as i32))
            .sum()
    }

    /// Manhattan (L1) distance: `∑ |a[i] - b[i]|`. Default impl is a
    /// straight scalar loop; SIMD backends override with `vabsq_f32`
    /// (NEON), `_mm_andnot_ps` + sign mask (SSE2/AVX2), or
    /// `_mm512_abs_ps` (AVX-512). Used by the new
    /// `DistanceMetric::Manhattan` collection setting.
    fn manhattan_distance(&self, a: &[f32], b: &[f32]) -> f32 {
        debug_assert_eq!(a.len(), b.len(), "Vectors must have same length");
        a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).sum()
    }

    /// Normalise `a` in-place to unit L2 norm. Returns silently
    /// without modifying `a` when the L2 norm is zero (a zero vector
    /// has no meaningful direction). Default impl runs a scalar
    /// `l2_norm` then a scalar divide; SIMD backends benefit because
    /// both passes vectorise.
    fn normalize_in_place(&self, a: &mut [f32]) {
        let norm = self.l2_norm(a);
        if norm == 0.0 || !norm.is_finite() {
            return;
        }
        let inv = 1.0 / norm;
        for x in a.iter_mut() {
            *x *= inv;
        }
    }

    /// Element-wise `a[i] += b[i]`. Default impl is a scalar loop;
    /// SIMD backends override using `_mm256_add_ps` / `vaddq_f32` /
    /// `f32x4_add`.
    fn add_assign(&self, a: &mut [f32], b: &[f32]) {
        debug_assert_eq!(a.len(), b.len(), "Vectors must have same length");
        for (x, y) in a.iter_mut().zip(b.iter()) {
            *x += *y;
        }
    }

    /// Element-wise `a[i] -= b[i]`. Default impl is a scalar loop.
    fn sub_assign(&self, a: &mut [f32], b: &[f32]) {
        debug_assert_eq!(a.len(), b.len(), "Vectors must have same length");
        for (x, y) in a.iter_mut().zip(b.iter()) {
            *x -= *y;
        }
    }

    /// Element-wise `a[i] *= s`. Default impl is a scalar loop.
    fn scale(&self, a: &mut [f32], s: f32) {
        for x in a.iter_mut() {
            *x *= s;
        }
    }

    /// Returns `Some((argmin, min))` over a non-empty slice, `None`
    /// for an empty slice. NaN values follow `f32::partial_cmp`
    /// semantics — they propagate as the larger element so a NaN
    /// never wins the argmin race.
    fn horizontal_min_index(&self, a: &[f32]) -> Option<(usize, f32)> {
        if a.is_empty() {
            return None;
        }
        let mut min_idx = 0usize;
        let mut min_val = a[0];
        for (i, &v) in a.iter().enumerate().skip(1) {
            if v < min_val {
                min_val = v;
                min_idx = i;
            }
        }
        Some((min_idx, min_val))
    }

    /// Quantize `src` into `dst` as 8-bit unsigned codes:
    /// `dst[i] = clamp(round((src[i] - offset) / scale), 0, levels - 1)`.
    ///
    /// Default implementation is a scalar loop. AVX2 (`x86::avx2`)
    /// and NEON (`aarch64::neon`) override this: the subtract,
    /// multiply-by-`1/scale`, and clamp/round steps vectorise
    /// cleanly, with a scalar tail for the remainder and a scalar
    /// narrow-to-`u8` step (see those modules' kernel docs for the
    /// exact rounding/NaN-handling rationale). Every other backend
    /// (SSE2, AVX-512F, AVX-512 VNNI, SVE, SVE2, WASM128) inherits
    /// this scalar default.
    ///
    /// Caller invariants: `dst.len() == src.len()`, `levels > 0`.
    /// Out-of-range inputs (NaN, infinite, magnitudes past the clamp
    /// boundary) are silently clamped.
    ///
    /// Edge case: `scale == 0.0` (constant-valued dataset where
    /// `max - min == 0`) writes all-zero codes. Mathematically every
    /// input maps to the same value, so any constant code is
    /// correct — 0 is the natural choice and it preserves the
    /// dst-buffer's initial state. This matches the semantics of
    /// the pre-7f scalar loop, which divided by zero (producing NaN)
    /// then clamped to 0 — without the panic risk that
    /// `1.0 / 0.0 → ∞ * (s - offset)` gives in debug builds.
    fn quantize_f32_to_u8(
        &self,
        src: &[f32],
        dst: &mut [u8],
        scale: f32,
        offset: f32,
        levels: u32,
    ) {
        debug_assert_eq!(dst.len(), src.len(), "Buffers must have same length");
        debug_assert!(levels > 0, "levels must be positive");
        if scale == 0.0 {
            // Constant-input short circuit; see method docs.
            for d in dst.iter_mut() {
                *d = 0;
            }
            return;
        }
        let inv_scale = 1.0 / scale;
        let max_level = (levels - 1) as f32;
        for (s, d) in src.iter().zip(dst.iter_mut()) {
            let normalised = (s - offset) * inv_scale;
            let clamped = normalised.clamp(0.0, max_level);
            *d = clamped.round() as u8;
        }
    }

    /// Dequantize `src` back to f32 as `dst[i] = offset + src[i] * scale`.
    ///
    /// Default implementation is a scalar loop. AVX2 (`x86::avx2`)
    /// and NEON (`aarch64::neon`) override this with a load-widen-
    /// multiply-add pattern (`u8 -> i32 -> f32`, widening is exact
    /// for the full `0..=255` range). The multiply and add stay two
    /// separate ops (not fused) so the rounding matches this scalar
    /// reference bit-for-bit. Every other backend (SSE2, AVX-512F,
    /// AVX-512 VNNI, SVE, SVE2, WASM128) inherits this scalar
    /// default.
    ///
    /// Caller invariants: `dst.len() == src.len()`.
    fn dequantize_u8_to_f32(&self, src: &[u8], dst: &mut [f32], scale: f32, offset: f32) {
        debug_assert_eq!(dst.len(), src.len(), "Buffers must have same length");
        for (s, d) in src.iter().zip(dst.iter_mut()) {
            *d = offset + (*s as f32) * scale;
        }
    }

    /// Diagnostic name. Must be a constant `&'static str`; surfaced
    /// by `dispatch::selected_backend_name()` and the startup log.
    fn name(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::SimdBackend;
    use crate::simd::scalar::ScalarBackend;

    const B: ScalarBackend = ScalarBackend;

    #[test]
    fn int8_dot_product_default() {
        assert_eq!(B.int8_dot_product(&[1, -2, 3], &[4, 5, -6]), 4 - 10 - 18);
        assert_eq!(B.int8_dot_product(&[], &[]), 0);
    }

    #[test]
    fn manhattan_distance_default() {
        assert_eq!(B.manhattan_distance(&[1.0, -2.0], &[4.0, 2.0]), 3.0 + 4.0);
        assert_eq!(B.manhattan_distance(&[], &[]), 0.0);
    }

    #[test]
    fn normalize_in_place_default_and_zero_vector() {
        let mut v = [3.0f32, 4.0];
        B.normalize_in_place(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        // Zero vector: no-op, never NaN.
        let mut z = [0.0f32, 0.0];
        B.normalize_in_place(&mut z);
        assert_eq!(z, [0.0, 0.0]);
    }

    #[test]
    fn elementwise_add_sub_scale_defaults() {
        let mut a = [1.0f32, 2.0, 3.0];
        B.add_assign(&mut a, &[10.0, 20.0, 30.0]);
        assert_eq!(a, [11.0, 22.0, 33.0]);
        B.sub_assign(&mut a, &[1.0, 2.0, 3.0]);
        assert_eq!(a, [10.0, 20.0, 30.0]);
        B.scale(&mut a, 0.5);
        assert_eq!(a, [5.0, 10.0, 15.0]);
    }

    #[test]
    fn horizontal_min_index_default() {
        assert_eq!(B.horizontal_min_index(&[]), None);
        assert_eq!(B.horizontal_min_index(&[7.0]), Some((0, 7.0)));
        assert_eq!(
            B.horizontal_min_index(&[3.0, -1.0, 2.0, -1.0]),
            Some((1, -1.0)), // first minimum wins
        );
    }

    #[test]
    fn quantize_default_clamps_rounds_and_short_circuits_constant_scale() {
        let src = [0.0f32, 3.0, 5.0, 510.0, -10.0, 9999.0];
        let mut dst = [0u8; 6];
        B.quantize_f32_to_u8(&src, &mut dst, 2.0, 0.0, 256);
        // round(1.5)=2 half-away-from-zero; out-of-range clamps to 0/255.
        assert_eq!(dst, [0, 2, 3, 255, 0, 255]);

        let mut constant = [77u8; 3];
        B.quantize_f32_to_u8(&[5.0, 5.0, 5.0], &mut constant, 0.0, 5.0, 256);
        assert_eq!(constant, [0, 0, 0]);
    }

    #[test]
    fn dequantize_default_is_offset_plus_code_times_scale() {
        let mut out = [0.0f32; 3];
        B.dequantize_u8_to_f32(&[0, 2, 255], &mut out, 2.0, 1.0);
        assert_eq!(out, [1.0, 5.0, 511.0]);
    }

    #[test]
    fn backend_name_is_stable() {
        assert_eq!(B.name(), "scalar");
    }
}
