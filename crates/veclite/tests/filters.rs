//! Payload-filter conformance (SPEC-006, gate G3): server-parity combination
//! semantics, index/scan equivalence (FLT-022), unsupported-feature rejection
//! (FLT-012), reserved-key enforcement (FLT-002), and pre-filter correctness
//! against a brute-force baseline (FLT-030/031).

use serde_json::json;
use veclite::{Condition, Filter, Metric, PayloadIndexKind, Point, Range, VecLite, VecLiteError};

fn opts() -> veclite::CollectionOptions {
    veclite::CollectionOptions::new(2, Metric::Euclidean).quantization(veclite::Quantization::None)
}

/// Corpus used across the semantics tests: distinct vectors, varied payloads,
/// and one point with no payload at all.
fn seed(c: &veclite::Collection) {
    c.upsert(Point::new("a", vec![0.0, 0.0]).payload(json!({"lang": "en", "year": 2024})))
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert(Point::new("b", vec![1.0, 0.0]).payload(json!({"lang": "pt", "year": 2020})))
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert(Point::new("c", vec![0.0, 1.0]).payload(json!({"lang": "en", "year": 2018})))
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert(Point::new("d", vec![1.0, 1.0]).payload(json!({"lang": "de", "year": 2024})))
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert(Point::new("e", vec![2.0, 2.0])) // no payload
        .unwrap_or_else(|e| panic!("{e}"));
}

/// Filtered ids for a query, sorted (the query vector returns all matches since
/// `limit` covers the corpus).
fn filtered_ids(c: &veclite::Collection, f: Filter) -> Vec<String> {
    let mut ids: Vec<String> = c
        .query(&[0.0, 0.0])
        .filter(f)
        .limit(100)
        .run()
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .map(|h| h.id)
        .collect();
    ids.sort();
    ids
}

#[test]
fn combination_semantics_match_expectations() {
    let db = VecLite::memory();
    let c = db
        .create_collection("docs", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    seed(&c);

    // must: lang=en AND year>=2021  → only a
    let f = Filter::new()
        .must(Condition::eq("lang", "en"))
        .must(Condition::range("year", Range::new().gte(2021.0)));
    assert_eq!(filtered_ids(&c, f), vec!["a"]);

    // should (OR): lang en or de → a, c, d
    let f = Filter::new()
        .should(Condition::eq("lang", "en"))
        .should(Condition::eq("lang", "de"));
    assert_eq!(filtered_ids(&c, f), vec!["a", "c", "d"]);

    // must_not lang=pt → everything except b (incl. the payload-less e)
    let f = Filter::new().must_not(Condition::eq("lang", "pt"));
    assert_eq!(filtered_ids(&c, f), vec!["a", "c", "d", "e"]);

    // exists year → a,b,c,d (not e)
    let f = Filter::new().must(Condition::exists("year"));
    assert_eq!(filtered_ids(&c, f), vec!["a", "b", "c", "d"]);

    // in lang [en, de] → a, c, d
    let f = Filter::new().must(Condition::in_("lang", vec!["en", "de"]));
    assert_eq!(filtered_ids(&c, f), vec!["a", "c", "d"]);

    // An empty filter behaves exactly like no filter at all (same fast path).
    let mut no_filter: Vec<String> = c
        .query(&[0.0, 0.0])
        .limit(100)
        .run()
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .map(|h| h.id)
        .collect();
    no_filter.sort();
    assert_eq!(filtered_ids(&c, Filter::new()), no_filter);
}

#[test]
fn portable_json_filter_matches_builder() {
    let db = VecLite::memory();
    let c = db
        .create_collection("docs", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    seed(&c);

    let doc = json!({
        "must": [
            {"key": "lang", "match": {"value": "en"}},
            {"key": "year", "range": {"gte": 2021}}
        ]
    });
    let f = Filter::from_json(&doc).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(filtered_ids(&c, f), vec!["a"]);
}

/// FLT-022: filtering on an indexed collection and an unindexed one returns
/// identical results — the index is an accelerator, not a gate.
#[test]
fn index_and_scan_agree() {
    let db = VecLite::memory();
    let indexed = db
        .create_collection(
            "indexed",
            opts()
                .payload_index("lang", PayloadIndexKind::Keyword)
                .payload_index("year", PayloadIndexKind::Integer),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let scanned = db
        .create_collection("scanned", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    seed(&indexed);
    seed(&scanned);

    let cases = [
        Filter::new()
            .must(Condition::eq("lang", "en"))
            .must(Condition::range("year", Range::new().gte(2021.0))),
        Filter::new().must(Condition::in_("lang", vec!["en", "de"])),
        Filter::new().must(Condition::exists("year")),
        Filter::new().must(Condition::range(
            "year",
            Range::new().gte(2019.0).lte(2023.0),
        )),
        // A key that is NOT indexed on either collection → both scan.
        Filter::new()
            .must(Condition::eq("lang", "en"))
            .must_not(Condition::eq("year", 2018i64)),
    ];
    for f in cases {
        assert_eq!(
            filtered_ids(&indexed, f.clone()),
            filtered_ids(&scanned, f),
            "indexed and scanned disagree"
        );
    }
}

/// FLT-030/031: with a selective filter, the filtered top-k equals the
/// brute-force filtered top-k (the pre-filter path is exact).
#[test]
fn prefilter_matches_bruteforce_topk() {
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "big",
            veclite::CollectionOptions::new(3, Metric::Euclidean)
                .quantization(veclite::Quantization::None)
                .payload_index("tag", PayloadIndexKind::Keyword),
        )
        .unwrap_or_else(|e| panic!("{e}"));

    // 1000 points; only ~1% carry tag=hot.
    let mut hot: Vec<(String, Vec<f32>)> = Vec::new();
    for i in 0..1000u32 {
        let v = vec![(i % 7) as f32, (i % 13) as f32, (i % 5) as f32];
        let is_hot = i % 100 == 0;
        let p = if is_hot {
            Point::new(format!("k{i}"), v.clone()).payload(json!({"tag": "hot"}))
        } else {
            Point::new(format!("k{i}"), v.clone()).payload(json!({"tag": "cold"}))
        };
        c.upsert(p).unwrap_or_else(|e| panic!("{e}"));
        if is_hot {
            hot.push((format!("k{i}"), v));
        }
    }

    let query = [1.0f32, 2.0, 3.0];
    let f = Filter::new().must(Condition::eq("tag", "hot"));
    let got: Vec<String> = c
        .query(&query)
        .filter(f)
        .limit(5)
        .run()
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .map(|h| h.id)
        .collect();

    // Brute-force baseline: the 5 nearest among the hot points.
    hot.sort_by(|a, b| {
        let da: f32 = a.1.iter().zip(&query).map(|(x, y)| (x - y).powi(2)).sum();
        let db_: f32 = b.1.iter().zip(&query).map(|(x, y)| (x - y).powi(2)).sum();
        da.total_cmp(&db_)
    });
    let expected: Vec<String> = hot.iter().take(5).map(|(id, _)| id.clone()).collect();
    assert_eq!(got, expected);
}

#[test]
fn reserved_underscore_keys_rejected() {
    let db = VecLite::memory();
    let c = db
        .create_collection("docs", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        c.upsert(Point::new("x", vec![0.0, 0.0]).payload(json!({"_text": "hi"}))),
        Err(VecLiteError::InvalidArgument(_))
    ));
    // A non-reserved key is fine.
    assert!(
        c.upsert(Point::new("y", vec![0.0, 0.0]).payload(json!({"text": "hi"})))
            .is_ok()
    );
}

#[test]
fn oversized_payload_rejected() {
    let db = VecLite::memory();
    let c = db
        .create_collection("docs", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    let big = "x".repeat(17 * 1024 * 1024); // > 16 MiB
    assert!(matches!(
        c.upsert(Point::new("x", vec![0.0, 0.0]).payload(json!({"blob": big}))),
        Err(VecLiteError::InvalidArgument(_))
    ));
}

/// A declared payload index is rebuilt from the loaded payloads on reopen (like
/// the HNSW graph), so filtered search keeps working after a restart.
#[test]
fn declared_index_survives_reopen() {
    let path = std::env::temp_dir().join(format!(
        "veclite-filter-{}-reopen.veclite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let mut wal = path.clone().into_os_string();
    wal.push("-wal");
    let _ = std::fs::remove_file(&wal);

    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection(
                "docs",
                opts().payload_index("lang", PayloadIndexKind::Keyword),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        seed(&c);
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    }

    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    let f = Filter::new().must(Condition::eq("lang", "en"));
    assert_eq!(filtered_ids(&c, f), vec!["a", "c"]);
    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&wal);
}

#[test]
fn unsupported_features_rejected_at_query() {
    let db = VecLite::memory();
    let c = db
        .create_collection("docs", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    seed(&c);

    // geo (only expressible via JSON)
    let geo = json!({"must": [{"key": "loc", "geo_radius": {"center": [0, 0], "radius": 1}}]});
    assert!(matches!(
        Filter::from_json(&geo),
        Err(VecLiteError::InvalidArgument(_))
    ));

    // nested-path key via the builder → rejected at query time
    let f = Filter::new().must(Condition::eq("meta.lang", "en"));
    assert!(matches!(
        c.query(&[0.0, 0.0]).filter(f).run(),
        Err(VecLiteError::InvalidArgument(_))
    ));
}
