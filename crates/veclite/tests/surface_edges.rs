//! Public-surface edge paths: payload limits and reserved keys, sparse-lane
//! search and validation (SPEC-007), hybrid error/extreme-alpha branches,
//! cosine guards, text-collection refit rules (SPEC-005).

#![cfg(not(target_arch = "wasm32"))]

use veclite::{
    CollectionOptions, Filter, Metric, Point, Quantization, SparseVector, VecLite, VecLiteError,
};

fn euclid(dim: usize) -> CollectionOptions {
    CollectionOptions::new(dim, Metric::Euclidean).quantization(Quantization::None)
}

fn sv(indices: Vec<u32>, values: Vec<f32>) -> SparseVector {
    SparseVector { indices, values }
}

#[test]
fn payload_rules_reserved_keys_and_size_limit() {
    let db = VecLite::memory();
    let c = db
        .create_collection("p", euclid(2))
        .unwrap_or_else(|e| panic!("{e}"));

    // Reserved `_`-prefixed top-level keys are rejected on the public path.
    let reserved = Point::new("r", vec![0.0, 0.0]).payload(serde_json::json!({"_text": "x"}));
    assert!(matches!(
        c.upsert(reserved),
        Err(VecLiteError::InvalidArgument(_))
    ));

    // Serialized payloads past 16 MiB are rejected (FLT-001).
    let big = "x".repeat(16 * 1024 * 1024 + 16);
    let oversized = Point::new("big", vec![0.0, 0.0]).payload(serde_json::json!({ "b": big }));
    assert!(matches!(
        c.upsert(oversized),
        Err(VecLiteError::InvalidArgument(_))
    ));
    assert_eq!(c.len(), 0, "rejected points must not be applied");
}

#[test]
fn sparse_lane_search_and_validation() {
    let db = VecLite::memory();
    let c = db
        .create_collection("s", euclid(2))
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert_batch(vec![
        Point::new("a", vec![0.0, 0.0])
            .sparse(sv(vec![1, 5], vec![1.0, 2.0]))
            .payload(serde_json::json!({"lang": "en"})),
        Point::new("b", vec![1.0, 0.0]).sparse(sv(vec![5, 9], vec![3.0, 1.0])),
        Point::new("dense-only", vec![2.0, 0.0]),
    ])
    .unwrap_or_else(|e| panic!("{e}"));

    // b scores 3*1=3 on index 5; a scores 2*1=2; dense-only has no lane.
    let q = sv(vec![5], vec![1.0]);
    let hits = c.search_sparse(&q, 10).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, "b");
    assert_eq!(hits[1].id, "a");

    // Zero-overlap queries return nothing (score 0 is filtered).
    let none = c
        .search_sparse(&sv(vec![100], vec![1.0]), 10)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(none.is_empty());

    // limit 0 and malformed sparse vectors are rejected.
    assert!(c.search_sparse(&q, 0).is_err());
    assert!(c.search_sparse(&sv(vec![5, 1], vec![1.0, 1.0]), 5).is_err()); // unsorted
    assert!(c.search_sparse(&sv(vec![1], vec![1.0, 2.0]), 5).is_err()); // len mismatch
    assert!(c.search_sparse(&sv(vec![1], vec![f32::NAN]), 5).is_err()); // non-finite
}

#[test]
fn hybrid_query_extremes_filters_and_errors() {
    let db = VecLite::memory();
    let c = db
        .create_collection("h", euclid(2))
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert_batch(vec![
        Point::new("near", vec![0.1, 0.0])
            .sparse(sv(vec![1], vec![0.1]))
            .payload(serde_json::json!({"lang": "en"})),
        Point::new("far", vec![9.0, 0.0])
            .sparse(sv(vec![1], vec![9.0]))
            .payload(serde_json::json!({"lang": "pt"})),
    ])
    .unwrap_or_else(|e| panic!("{e}"));

    let q = [0.0f32, 0.0];
    let s = sv(vec![1], vec![1.0]);

    // alpha=1: dense-only ranking; alpha=0: sparse-only ranking.
    let dense_only = c
        .hybrid_query()
        .dense(&q)
        .sparse(&s)
        .alpha(1.0)
        .limit(2)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(dense_only[0].id, "near");
    let sparse_only = c
        .hybrid_query()
        .dense(&q)
        .sparse(&s)
        .alpha(0.0)
        .limit(2)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(sparse_only[0].id, "far");

    // Filter applies across both lanes.
    let filtered = c
        .hybrid_query()
        .dense(&q)
        .sparse(&s)
        .limit(10)
        .filter(
            Filter::from_json(&serde_json::json!({"must":[{"key":"lang","match":{"value":"en"}}]}))
                .unwrap_or_else(|e| panic!("{e}")),
        )
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, "near");

    // Out-of-range alpha CLAMPS to [0, 1] (documented): 1.5 behaves as
    // dense-only, -0.1 as sparse-only.
    let clamped_hi = c
        .hybrid_query()
        .dense(&q)
        .sparse(&s)
        .alpha(1.5)
        .limit(1)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(clamped_hi[0].id, "near");
    let clamped_lo = c
        .hybrid_query()
        .dense(&q)
        .sparse(&s)
        .alpha(-0.1)
        .limit(1)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(clamped_lo[0].id, "far");

    // Error branches: no lane; limit 0; malformed sparse.
    assert!(c.hybrid_query().limit(5).run().is_err());
    assert!(c.hybrid_query().dense(&q).limit(0).run().is_err());
    let bad = sv(vec![2, 1], vec![1.0, 1.0]);
    assert!(c.hybrid_query().sparse(&bad).limit(1).run().is_err());
}

#[test]
fn cosine_guards_zero_vectors_and_normalizes() {
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "cos",
            CollectionOptions::new(2, Metric::Cosine).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    // Zero vectors are invalid under cosine, at ingest and at query.
    assert!(c.upsert(Point::new("z", vec![0.0, 0.0])).is_err());
    c.upsert(Point::new("x", vec![3.0, 4.0]))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(c.search(&[0.0, 0.0], 1).is_err());

    // Stored vectors are unit-normalized at ingest (CORE-014).
    let got = c
        .get("x")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("x missing"));
    assert!((got.vector[0] - 0.6).abs() < 1e-6);
    assert!((got.vector[1] - 0.8).abs() < 1e-6);
    let hits = c.search(&[3.0, 4.0], 1).unwrap_or_else(|e| panic!("{e}"));
    assert!((hits[0].score - 1.0).abs() < 1e-5, "self-similarity ~1");
}

#[test]
fn text_api_rules_and_refit() {
    let db = VecLite::memory();

    // Text API on a BYO collection is a mode error, as is BYO sparse on an
    // auto-embed collection (HYB-002); refit needs an embedder.
    let byo = db
        .create_collection("byo", euclid(2))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(byo.upsert_text("t", "hello").is_err());
    assert!(byo.search_text("hello", 1).is_err());
    assert!(byo.refit().is_err());

    let text = db
        .create_collection("txt", CollectionOptions::auto_embed("bm25", 64))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        text.upsert(Point::new("s", vec![0.0; 64]).sparse(sv(vec![1], vec![1.0])))
            .is_err()
    );

    text.upsert_text_batch(vec![
        (
            "cats".into(),
            "cats are small furry animals that meow".into(),
            Some(serde_json::json!({"kind": "animal"})),
        ),
        (
            "cars".into(),
            "cars are fast vehicles with engines".into(),
            None,
        ),
    ])
    .unwrap_or_else(|e| panic!("{e}"));
    // Explicit refit is idempotent with the lazy one.
    text.refit().unwrap_or_else(|e| panic!("{e}"));
    let hits = text
        .search_text("furry animals", 1)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "cats");
    // User payload survives re-embedding.
    assert_eq!(
        hits[0].payload.as_ref().and_then(|p| p.get("kind")),
        Some(&serde_json::json!("animal"))
    );
    assert!(text.search_text("anything", 0).is_err());
}

#[test]
fn scroll_cursor_survives_deletion_of_the_cursor_id() {
    let db = VecLite::memory();
    let c = db
        .create_collection("sc", euclid(2))
        .unwrap_or_else(|e| panic!("{e}"));
    let points: Vec<Point> = (0..6)
        .map(|i| Point::new(format!("k{i}"), vec![i as f32, 0.0]))
        .collect();
    c.upsert_batch(points).unwrap_or_else(|e| panic!("{e}"));

    let page1 = c.scroll(None, 2, None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(page1.points.len(), 2);
    let cursor = page1.next_cursor.unwrap_or_else(|| panic!("cursor"));

    // Delete the id the cursor points at: the next page must resume after
    // the slot it occupied (SPEC-004 API-022), skipping nothing else.
    assert!(c.delete(&cursor).unwrap_or_else(|e| panic!("{e}")));
    let page2 = c
        .scroll(Some(&cursor), 10, None)
        .unwrap_or_else(|e| panic!("{e}"));
    let ids: Vec<&str> = page2.points.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["k2", "k3", "k4", "k5"]);
    assert!(page2.next_cursor.is_none());

    // Filtered scroll (FLT-032) + limit 0 rejection.
    let filt = Filter::from_json(&serde_json::json!({"must":[{"key":"missing","exists":true}]}))
        .unwrap_or_else(|e| panic!("{e}"));
    let empty = c
        .scroll(None, 5, Some(&filt))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(empty.points.is_empty());
    assert!(c.scroll(None, 0, None).is_err());
}
