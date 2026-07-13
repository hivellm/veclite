//! Vendored from hivellm/vectorizer vectorizer-core@3.5.0
//! (crates/vectorizer-core/src/simd/dispatch.rs), Apache-2.0. Adapted:
//! scalar-only (ISA backends deferred), crate paths localized.
//!
//! Runtime backend dispatch.
//!
//! Picks a `SimdBackend` once at first use, caches it in a
//! `OnceLock`, and hands `&'static dyn SimdBackend` references to
//! every call site.
//!
//! This vendor slice resolves unconditionally to `ScalarBackend` —
//! per-ISA selection ladders (AVX2, NEON, ...) and the
//! `VECTORIZER_SIMD_BACKEND` env override are deferred to a follow-up
//! task. The public function shapes (`backend`, `selected_backend_name`,
//! and the internal `select_backend`) are kept stable so adding an
//! ISA branch later is a pure addition, not a rewrite.

use std::sync::OnceLock;

use super::backend::SimdBackend;
use super::scalar::ScalarBackend;

/// Returns the best backend for the running build, picking it on the
/// first call and reusing the same `&'static dyn SimdBackend` from
/// then on. Safe to call from multiple threads.
pub fn backend() -> &'static dyn SimdBackend {
    static CACHED: OnceLock<&'static dyn SimdBackend> = OnceLock::new();
    *CACHED.get_or_init(select_backend)
}

/// Diagnostic helper. Returns the `name()` of the backend the
/// dispatcher chose. Calling this also primes the `OnceLock` if it
/// hasn't been hit yet.
pub fn selected_backend_name() -> &'static str {
    backend().name()
}

/// Selection logic. Scalar-only in this vendor slice — every target
/// resolves to `ScalarBackend` regardless of the `simd` Cargo feature
/// or CPU capability. Per-ISA priority ladders are a follow-up task.
fn select_backend() -> &'static dyn SimdBackend {
    &ScalarBackend
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_is_stable_across_calls() {
        // `OnceLock` semantics: every call returns the same pointer.
        let a = backend();
        let b = backend();
        let a_addr = a as *const dyn SimdBackend as *const () as usize;
        let b_addr = b as *const dyn SimdBackend as *const () as usize;
        assert_eq!(a_addr, b_addr);
    }

    #[test]
    fn selected_backend_name_is_scalar() {
        // Scalar-only in this vendor slice; ISA backends are a
        // future addition (see module docs).
        assert_eq!(selected_backend_name(), "scalar");
    }
}
