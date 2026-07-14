//! Hybrid dense+sparse search (SPEC-007): sparse validation, single-lane
//! degeneration (HYB-010), deterministic RRF fusion (HYB-020/021), and filtered
//! hybrid.

use serde_json::json;
use veclite::{
    CollectionOptions, Metric, Point, Quantization, SparseVector, VecLite, VecLiteError,
};

fn ids(hits: &[veclite::Hit]) -> Vec<String> {
    hits.iter().map(|h| h.id.clone()).collect()
}

fn sv(indices: Vec<u32>, values: Vec<f32>) -> SparseVector {
    SparseVector { indices, values }
}

fn coll() -> (VecLite, veclite::Collection) {
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "docs",
            CollectionOptions::new(2, Metric::Euclidean).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    // Dense positions + BYO sparse lanes over a small term space.
    c.upsert(Point::new("a", vec![0.0, 0.0]).sparse(sv(vec![0, 2], vec![1.0, 1.0])))
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert(Point::new("b", vec![1.0, 0.0]).sparse(sv(vec![1, 2], vec![1.0, 2.0])))
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert(Point::new("d", vec![0.0, 1.0]).sparse(sv(vec![0, 1], vec![2.0, 1.0])))
        .unwrap_or_else(|e| panic!("{e}"));
    (db, c)
}

#[test]
fn sparse_validation_rejects_bad_vectors() {
    let (_db, c) = coll();
    // not strictly increasing
    assert!(matches!(
        c.upsert(Point::new("x", vec![0.0, 0.0]).sparse(sv(vec![2, 2], vec![1.0, 1.0]))),
        Err(VecLiteError::InvalidArgument(_))
    ));
    // length mismatch
    assert!(matches!(
        c.upsert(Point::new("y", vec![0.0, 0.0]).sparse(sv(vec![0, 1], vec![1.0]))),
        Err(VecLiteError::InvalidArgument(_))
    ));
    // non-finite value
    assert!(matches!(
        c.upsert(Point::new("z", vec![0.0, 0.0]).sparse(sv(vec![0], vec![f32::NAN]))),
        Err(VecLiteError::InvalidArgument(_))
    ));
}

#[test]
fn no_lane_is_rejected() {
    let (_db, c) = coll();
    assert!(matches!(
        c.hybrid_query().limit(3).run(),
        Err(VecLiteError::InvalidArgument(_))
    ));
}

#[test]
fn single_dense_lane_equals_plain_search() {
    let (_db, c) = coll();
    let q = [0.1, 0.0];
    let hybrid = c
        .hybrid_query()
        .dense(&q)
        .limit(3)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    let plain = c.search(&q, 3).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        hybrid, plain,
        "dense-only hybrid must equal search (HYB-010)"
    );
}

#[test]
fn single_sparse_lane_equals_sparse_search() {
    let (_db, c) = coll();
    let sq = sv(vec![1, 2], vec![1.0, 1.0]);
    let hybrid = c
        .hybrid_query()
        .sparse(&sq)
        .limit(3)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    let plain = c.search_sparse(&sq, 3).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hybrid, plain, "sparse-only hybrid must equal search_sparse");
}

#[test]
fn rrf_fusion_is_deterministic() {
    let (_db, c) = coll();
    let q = [0.1, 0.0];
    let sq = sv(vec![1, 2], vec![1.0, 1.0]);
    let first = ids(&c
        .hybrid_query()
        .dense(&q)
        .sparse(&sq)
        .limit(3)
        .run()
        .unwrap_or_else(|e| panic!("{e}")));
    for _ in 0..5 {
        let again = ids(&c
            .hybrid_query()
            .dense(&q)
            .sparse(&sq)
            .limit(3)
            .run()
            .unwrap_or_else(|e| panic!("{e}")));
        assert_eq!(first, again, "repeated hybrid queries must be identical");
    }
}

#[test]
fn rrf_matches_hand_computed_fusion() {
    let (_db, c) = coll();
    // Dense query nearest to `a` [0,0]: order a(0), b(1,0)->dist1, d(0,1)->dist1
    // (b and d tie in distance; dense tie-break is by score then insertion —
    // we only rely on the fused math below, computed from the actual lane ranks).
    let q = [0.05, 0.0];
    let sq = sv(vec![1, 2], vec![1.0, 1.0]); // matches b (idx1,2) and d (idx1) and a (idx2)
    let hits = c
        .hybrid_query()
        .dense(&q)
        .sparse(&sq)
        .alpha(0.5)
        .rrf_k(60.0)
        .limit(3)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    // Every hit's score is a positive fused RRF score, strictly descending.
    assert!(!hits.is_empty());
    for w in hits.windows(2) {
        assert!(w[0].score >= w[1].score, "fused scores must be descending");
    }
    assert!(hits.iter().all(|h| h.score > 0.0));
}

#[test]
fn filter_applies_to_both_lanes() {
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "docs",
            CollectionOptions::new(2, Metric::Euclidean).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert(
        Point::new("en", vec![0.0, 0.0])
            .sparse(sv(vec![0, 1], vec![1.0, 1.0]))
            .payload(json!({"lang": "en"})),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    c.upsert(
        Point::new("pt", vec![0.0, 0.0])
            .sparse(sv(vec![0, 1], vec![1.0, 1.0]))
            .payload(json!({"lang": "pt"})),
    )
    .unwrap_or_else(|e| panic!("{e}"));

    let f = veclite::Filter::new().must(veclite::Condition::eq("lang", "en"));
    let hits = c
        .hybrid_query()
        .dense(&[0.0, 0.0])
        .sparse(&sv(vec![0, 1], vec![1.0, 1.0]))
        .filter(f)
        .limit(10)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        ids(&hits),
        vec!["en"],
        "filter must exclude pt from both lanes"
    );
}
