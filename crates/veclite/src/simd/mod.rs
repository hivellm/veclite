//! Vendored from hivellm/vectorizer vectorizer-core@3.5.0
//! (crates/vectorizer-core/src/simd/mod.rs), Apache-2.0. Adapted:
//! scalar-only (ISA backends deferred), crate paths localized.
//!
//! SIMD-accelerated vector primitives with runtime CPU dispatch.
//!
//! This vendor slice ships the `backend::SimdBackend` trait, the
//! `dispatch::backend` cache, and the `scalar::ScalarBackend`
//! correctness oracle. Per-ISA backends (AVX2, NEON, ...) are
//! deferred to a follow-up task — `dispatch::backend()` always
//! resolves to the scalar backend today, and the public function
//! shapes are kept stable so adding an ISA branch later is a pure
//! addition.
//!
//! ## Public API
//!
//! Most callers want the convenience functions exported from this
//! module — they hide the backend lookup behind a normal function
//! call. Use the trait directly only if you want to bind to a
//! specific backend (testing, benchmarking).

pub mod backend;
pub mod dispatch;
pub mod scalar;

pub use backend::SimdBackend;
pub use dispatch::backend;

// ── Convenience functions ────────────────────────────────────────────

/// Sum of pairwise products: `∑ a[i] * b[i]`.
///
/// Routes through the cached [`dispatch::backend`] — first call
/// resolves the per-CPU backend, subsequent calls are a single
/// indirect call. Mismatched-length slices are a debug-asserted
/// caller bug.
#[inline]
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    backend().dot_product(a, b)
}

/// `sqrt(∑ (a[i] - b[i])²)` — Euclidean distance between two equal-
/// length vectors. If you need the squared distance (cheaper, no
/// `sqrt`), call [`euclidean_distance_squared`] directly.
#[inline]
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    backend().euclidean_distance_squared(a, b).sqrt()
}

/// `∑ (a[i] - b[i])²` — Euclidean SQUARED distance. Use this when
/// comparing distances; the `sqrt` is monotonic so the ranking is
/// preserved and you save the call.
#[inline]
pub fn euclidean_distance_squared(a: &[f32], b: &[f32]) -> f32 {
    backend().euclidean_distance_squared(a, b)
}

/// Cosine similarity ASSUMING pre-normalised inputs — implemented as
/// a clamped dot product (`dot.clamp(-1.0, 1.0)`). If your vectors
/// are not unit-length, normalise first.
#[inline]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    backend().cosine_similarity(a, b)
}

/// `sqrt(∑ a[i]²)` — L2 norm of a single vector.
#[inline]
pub fn l2_norm(a: &[f32]) -> f32 {
    backend().l2_norm(a)
}

/// Normalise `a` in-place to unit L2 norm. No-op on a zero vector
/// (a zero vector has no meaningful direction; the alternative is
/// returning NaN, which propagates badly through downstream math).
#[inline]
pub fn normalize_in_place(a: &mut [f32]) {
    backend().normalize_in_place(a);
}

/// Quantize `src` to 8-bit unsigned codes in `dst`:
/// `dst[i] = clamp(round((src[i] - offset) / scale), 0, levels - 1)`.
/// See [`SimdBackend::quantize_f32_to_u8`] for invariants.
#[inline]
pub fn quantize_f32_to_u8(src: &[f32], dst: &mut [u8], scale: f32, offset: f32, levels: u32) {
    backend().quantize_f32_to_u8(src, dst, scale, offset, levels);
}

/// Dequantize `src` back to f32 as `dst[i] = offset + src[i] * scale`.
/// See [`SimdBackend::dequantize_u8_to_f32`].
#[inline]
pub fn dequantize_u8_to_f32(src: &[u8], dst: &mut [f32], scale: f32, offset: f32) {
    backend().dequantize_u8_to_f32(src, dst, scale, offset);
}
