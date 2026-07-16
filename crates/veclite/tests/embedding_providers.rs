//! Provider-level coverage for every built-in sparse embedder (SPEC-005): the
//! empty-corpus fit, in-vocabulary and all-out-of-vocabulary embeds (the
//! `norm > 0` normalization branch and its zero-vector twin), short-text
//! n-gram handling, `dimension()`, and the incremental `add_document` path.

use veclite::build_provider;

fn exercise(name: &str) {
    let dim = 64;
    let mut p = build_provider(name, dim).unwrap_or_else(|e| panic!("{name}: {e}"));

    // Fitting an empty corpus must not divide-by-zero (avg length = 0).
    p.fit(&[])
        .unwrap_or_else(|e| panic!("{name} empty fit: {e}"));

    // Real corpus, then assert the trait surface.
    let corpus = [
        "the quick brown fox",
        "a lazy dog sleeps",
        "quick foxes run fast",
    ];
    p.fit(&corpus).unwrap_or_else(|e| panic!("{name} fit: {e}"));
    assert_eq!(p.dimension(), dim, "{name} dimension");

    // In-vocabulary text → a finite, unit-ish (normalized) vector.
    let v = p
        .embed("quick fox")
        .unwrap_or_else(|e| panic!("{name} embed: {e}"));
    assert_eq!(v.len(), dim, "{name} embed len");
    assert!(v.iter().all(|x| x.is_finite()), "{name} finite");
    assert!(v.iter().any(|&x| x != 0.0), "{name} in-vocab is non-zero");

    // All-out-of-vocabulary text → the zero-norm branch yields a zero vector,
    // never NaN.
    let z = p
        .embed("zzzzz wwwww qqqqq")
        .unwrap_or_else(|e| panic!("{name} oov embed: {e}"));
    assert_eq!(z.len(), dim);
    assert!(z.iter().all(|x| x.is_finite()), "{name} oov finite");

    // Very short text (shorter than a char n-gram window) still embeds.
    let s = p
        .embed("a")
        .unwrap_or_else(|e| panic!("{name} short embed: {e}"));
    assert_eq!(s.len(), dim);

    // Incremental fold-in (add_document) updates state without error.
    p.add_document("another fresh document about foxes");
    let after = p
        .embed("fox")
        .unwrap_or_else(|e| panic!("{name} post-add embed: {e}"));
    assert_eq!(after.len(), dim);
}

#[test]
fn bm25_provider_surface() {
    exercise("bm25");
}

#[test]
fn bow_provider_surface() {
    exercise("bow");
}

#[test]
fn char_ngram_provider_surface() {
    exercise("char_ngram");
}

#[test]
fn tfidf_provider_surface() {
    exercise("tfidf");
}
