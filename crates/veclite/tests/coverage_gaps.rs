//! Targeted coverage for reachable-but-previously-unexercised public paths:
//! hybrid projection toggles, nested filter composition, Binary quantization,
//! and the `Embedder::embed_batch` default. Behavioral, not coverage theater —
//! each asserts the documented result.

use veclite::{
    CollectionOptions, Condition, Filter, Metric, Point, Quantization, SparseVector, VecLite,
    build_provider,
};

#[test]
fn hybrid_projection_toggles() {
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "h",
            CollectionOptions::new(1, Metric::Euclidean).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    for (i, id) in ["a", "b", "c"].iter().enumerate() {
        c.upsert(
            Point::new((*id).to_owned(), vec![i as f32]).sparse(SparseVector {
                indices: vec![0],
                values: vec![(3 - i) as f32],
            }),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    // with_payload(false) + with_vector(true): hits both builder methods and the
    // projection branches.
    let hits = c
        .hybrid_query()
        .dense(&[0.0])
        .sparse(&SparseVector {
            indices: vec![0],
            values: vec![1.0],
        })
        .with_payload(false)
        .with_vector(true)
        .limit(3)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits.len(), 3);
    assert!(hits[0].payload.is_none(), "payload suppressed");
    assert!(hits[0].vector.is_some(), "vector included");
}

#[test]
fn binary_quantization_round_trips_through_a_collection() {
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "bq",
            CollectionOptions::new(8, Metric::Euclidean).quantization(Quantization::Binary),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    for i in 0..16u32 {
        let v: Vec<f32> = (0..8).map(|d| ((i >> d) & 1) as f32).collect();
        c.upsert(Point::new(format!("k{i}"), v))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    // A query returns its nearest neighbours under the binary codes.
    let q = vec![1.0f32; 8];
    let hits = c.query(&q).limit(1).run().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "k15"); // all-ones vector is the exact match
}

#[test]
fn nested_filter_composition_parses_and_validates() {
    // A condition that is itself a clause-set → Condition::Nested (from_json).
    let doc = serde_json::json!({
        "must": [
            { "must": [ { "key": "lang", "match": { "value": "en" } } ],
              "should": [ { "key": "year", "range": { "gte": 2020 } } ] }
        ]
    });
    let f = Filter::from_json(&doc).unwrap_or_else(|e| panic!("{e}"));
    // Applying it selects points satisfying the inner clause set.
    let en_2021 = serde_json::json!({ "lang": "en", "year": 2021 });
    let en_2000 = serde_json::json!({ "lang": "en", "year": 2000 });
    assert!(f.matches(Some(&en_2021)));
    assert!(!f.matches(Some(&en_2000))); // should-clause fails
}

#[test]
fn nested_path_keys_are_rejected() {
    // FLT-012: a dotted (nested-path) key is rejected, never silently ignored.
    let doc = serde_json::json!({ "must": [ { "key": "a.b", "match": { "value": 1 } } ] });
    assert!(Filter::from_json(&doc).is_err());
}

#[test]
fn nested_condition_matches_via_builder() {
    // A builder-constructed nested clause recurses into the inner filter.
    let inner = Filter::new().must(Condition::eq("lang", "en"));
    let f = Filter::new().must(Condition::Nested(Box::new(inner)));
    assert!(f.matches(Some(&serde_json::json!({ "lang": "en" }))));
    assert!(!f.matches(Some(&serde_json::json!({ "lang": "pt" }))));
}

#[test]
fn embedder_embed_batch_default() {
    // The trait's default `embed_batch` maps `embed` over the inputs; providers
    // don't override it, so exercising it here covers that default.
    let mut provider = build_provider("bm25", 64).unwrap_or_else(|e| panic!("{e}"));
    let corpus = ["the quick brown fox", "a lazy dog", "quick foxes run"];
    provider.fit(&corpus).unwrap_or_else(|e| panic!("{e}"));
    let batch = provider
        .embed_batch(&["quick fox", "lazy dog"])
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].len(), 64);
    // Same as calling embed one at a time.
    let single = provider
        .embed("quick fox")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(batch[0], single);
}
