//! Public search API (SPEC-004 §4–5): per-metric ordering (CORE-035), the
//! query-builder option matrix, and input-validation edge cases.

use veclite::{Collection, CollectionOptions, Metric, Point, Quantization, VecLite, VecLiteError};

fn collection(dim: usize, metric: Metric) -> Collection {
    VecLite::memory()
        .create_collection(
            "c",
            CollectionOptions::new(dim, metric).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"))
}

fn upsert(c: &Collection, id: &str, vector: Vec<f32>) {
    c.upsert(Point::new(id, vector))
        .unwrap_or_else(|e| panic!("{e}"));
}

fn ids(hits: &[veclite::Hit]) -> Vec<String> {
    hits.iter().map(|h| h.id.clone()).collect()
}

// ── Task 2.1 — ordering per metric (CORE-035) ────────────────────────────

#[test]
fn cosine_orders_by_descending_similarity() {
    let c = collection(2, Metric::Cosine);
    upsert(&c, "a", vec![1.0, 0.0]);
    upsert(&c, "c", vec![1.0, 1.0]);
    upsert(&c, "b", vec![0.0, 1.0]);

    let hits = c.search(&[1.0, 0.1], 3).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(ids(&hits), ["a", "c", "b"]);
    for w in hits.windows(2) {
        assert!(w[0].score >= w[1].score, "cosine scores must be descending");
    }
    // Cosine similarity is in [-1, 1].
    assert!(hits[0].score <= 1.0 + 1e-6 && hits[0].score > 0.9);
}

#[test]
fn dot_product_orders_by_descending_similarity() {
    // DotProduct has no HNSW index (DistDot panics on unnormalized vectors),
    // so this exercises the exact brute-force path.
    let c = collection(2, Metric::DotProduct);
    upsert(&c, "a", vec![2.0, 0.0]);
    upsert(&c, "c", vec![1.0, 1.0]);
    upsert(&c, "b", vec![0.0, 1.0]);

    let hits = c.search(&[1.0, 0.0], 3).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(ids(&hits), ["a", "c", "b"]);
    assert!((hits[0].score - 2.0).abs() < 1e-6, "dot(query,a) = 2");
    for w in hits.windows(2) {
        assert!(w[0].score >= w[1].score, "dot scores must be descending");
    }
}

#[test]
fn euclidean_orders_by_ascending_distance() {
    let c = collection(2, Metric::Euclidean);
    upsert(&c, "a", vec![0.0, 0.0]);
    upsert(&c, "b", vec![1.0, 0.0]);
    upsert(&c, "c", vec![5.0, 5.0]);

    let hits = c.search(&[0.1, 0.0], 3).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(ids(&hits), ["a", "b", "c"]);
    for w in hits.windows(2) {
        assert!(
            w[0].score <= w[1].score,
            "euclidean scores must be ascending"
        );
    }
    assert!(
        (hits[0].score - 0.1).abs() < 1e-6,
        "distance(query,a) = 0.1"
    );
}

// ── Task 2.2 — query-builder option matrix ───────────────────────────────

#[test]
fn builder_defaults_include_payload_exclude_vector() {
    let c = collection(2, Metric::Euclidean);
    c.upsert(Point::new("a", vec![1.0, 2.0]).payload(serde_json::json!({"k": 1})))
        .unwrap_or_else(|e| panic!("{e}"));

    let hits = c.query(&[1.0, 2.0]).run().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].payload, Some(serde_json::json!({"k": 1})));
    assert_eq!(hits[0].vector, None);
}

#[test]
fn builder_projection_overrides() {
    let c = collection(2, Metric::Euclidean);
    c.upsert(Point::new("a", vec![3.0, 4.0]).payload(serde_json::json!({"k": 1})))
        .unwrap_or_else(|e| panic!("{e}"));

    let no_payload = c
        .query(&[3.0, 4.0])
        .with_payload(false)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(no_payload[0].payload, None);

    let with_vector = c
        .query(&[3.0, 4.0])
        .with_vector(true)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(with_vector[0].vector, Some(vec![3.0, 4.0]));
}

#[test]
fn builder_limit_and_ef_search() {
    let c = collection(2, Metric::Euclidean);
    for i in 0..20 {
        #[allow(clippy::cast_precision_loss)]
        upsert(&c, &format!("v{i}"), vec![i as f32, 0.0]);
    }
    // Explicit limit caps the result count.
    let two = c
        .query(&[0.0, 0.0])
        .limit(2)
        .ef_search(64)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(two.len(), 2);
    // Default limit is 10 (SPEC-004 §5).
    let default = c.query(&[0.0, 0.0]).run().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(default.len(), 10);
}

// ── Task 2.3 — edge cases ────────────────────────────────────────────────

#[test]
fn limit_zero_is_rejected() {
    let c = collection(2, Metric::Euclidean);
    upsert(&c, "a", vec![1.0, 2.0]);
    let Err(err) = c.search(&[1.0, 2.0], 0) else {
        panic!("limit 0 must be rejected")
    };
    assert!(matches!(err, VecLiteError::InvalidArgument(_)));
}

#[test]
fn limit_above_live_count_returns_all_live() {
    let c = collection(2, Metric::Euclidean);
    upsert(&c, "a", vec![1.0, 0.0]);
    upsert(&c, "b", vec![0.0, 1.0]);
    let hits = c.search(&[1.0, 1.0], 100).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits.len(), 2);
}

#[test]
fn empty_collection_returns_no_hits() {
    let c = collection(3, Metric::Euclidean);
    let hits = c
        .search(&[1.0, 2.0, 3.0], 10)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(hits.is_empty());
}

#[test]
fn wrong_query_dimension_is_rejected() {
    let c = collection(3, Metric::Euclidean);
    upsert(&c, "a", vec![1.0, 2.0, 3.0]);
    let Err(err) = c.search(&[1.0, 2.0], 5) else {
        panic!("dimension mismatch must be rejected")
    };
    assert!(matches!(
        err,
        VecLiteError::DimensionMismatch {
            expected: 3,
            got: 2
        }
    ));
}

#[test]
fn out_of_range_ef_search_is_rejected() {
    let c = collection(2, Metric::Euclidean);
    upsert(&c, "a", vec![1.0, 2.0]);
    let Err(err) = c.query(&[1.0, 2.0]).ef_search(0).run() else {
        panic!("ef_search 0 must be rejected")
    };
    assert!(matches!(err, VecLiteError::InvalidArgument(_)));
    let Err(err) = c.query(&[1.0, 2.0]).ef_search(5000).run() else {
        panic!("ef_search above 4096 must be rejected")
    };
    assert!(matches!(err, VecLiteError::InvalidArgument(_)));
}

#[test]
fn non_finite_query_is_rejected() {
    let c = collection(2, Metric::Euclidean);
    upsert(&c, "a", vec![1.0, 2.0]);
    let Err(err) = c.search(&[f32::NAN, 1.0], 5) else {
        panic!("NaN query must be rejected")
    };
    assert!(matches!(err, VecLiteError::InvalidArgument(_)));
}
