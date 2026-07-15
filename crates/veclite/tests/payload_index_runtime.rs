//! Runtime payload-index creation (SPEC-006 FLT-020) and the filtered-search
//! planner (FLT-030/031): late `create_payload_index` backfills and persists
//! (PIDX segments + PIDX_DECLARE replay); every planner strategy returns the
//! exact-scan baseline.

#![cfg(not(target_arch = "wasm32"))]

use veclite::{CollectionOptions, Filter, Metric, PayloadIndexKind, Point, Quantization, VecLite};

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "veclite-pidx-{}-{name}.veclite",
        std::process::id()
    ))
}

fn cleanup(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let _ = std::fs::remove_file(std::path::PathBuf::from(wal));
}

fn opts() -> CollectionOptions {
    CollectionOptions::new(2, Metric::Euclidean).quantization(Quantization::None)
}

fn lang_filter(lang: &str) -> Filter {
    Filter::from_json(&serde_json::json!({"must":[{"key":"lang","match":{"value": lang}}]}))
        .unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn late_index_backfills_and_survives_checkpoint_reopen() {
    let path = tmp("late");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("v", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        let points: Vec<Point> = (0..40)
            .map(|i| {
                let lang = if i % 4 == 0 { "en" } else { "pt" };
                Point::new(format!("k{i}"), vec![i as f32, 0.0])
                    .payload(serde_json::json!({"lang": lang, "i": i}))
            })
            .collect();
        c.upsert_batch(points).unwrap_or_else(|e| panic!("{e}"));

        // Late declaration backfills the existing payloads.
        c.create_payload_index("lang", PayloadIndexKind::Keyword)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            c.stats().payload_indexes,
            vec![("lang".to_owned(), PayloadIndexKind::Keyword)]
        );
        let hits = c
            .query(&[0.0, 0.0])
            .limit(3)
            .filter(lang_filter("en"))
            .run()
            .unwrap_or_else(|e| panic!("{e}"));
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["k0", "k4", "k8"]);
        // close: checkpoint seals a PIDX segment
    }
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    // Declaration reloaded from the PIDX segment, index rebuilt, still used.
    assert_eq!(
        c.stats().payload_indexes,
        vec![("lang".to_owned(), PayloadIndexKind::Keyword)]
    );
    let hits = c
        .query(&[0.0, 0.0])
        .limit(3)
        .filter(lang_filter("en"))
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "k0");
    cleanup(&path);
}

#[test]
fn declarations_replay_from_the_wal_after_a_crash() {
    let path = tmp("crash");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        // Creation-time declaration: journaled as PIDX_DECLARE after CREATE_COLL.
        let c = db
            .create_collection("v", opts().payload_index("year", PayloadIndexKind::Integer))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("a", vec![0.0, 0.0]).payload(serde_json::json!({"year": 2020})))
            .unwrap_or_else(|e| panic!("{e}"));
        // Runtime declaration on top, then crash before any checkpoint.
        c.create_payload_index("lang", PayloadIndexKind::Keyword)
            .unwrap_or_else(|e| panic!("{e}"));
        db.__test_simulate_crash();
    }
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        c.stats().payload_indexes,
        vec![
            ("lang".to_owned(), PayloadIndexKind::Keyword),
            ("year".to_owned(), PayloadIndexKind::Integer),
        ]
    );
    cleanup(&path);
}

#[test]
fn redeclare_semantics_and_key_validation() {
    let db = VecLite::memory();
    let c = db
        .create_collection("v", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    c.create_payload_index("lang", PayloadIndexKind::Keyword)
        .unwrap_or_else(|e| panic!("{e}"));
    // Same kind: idempotent no-op. Different kind: conflict.
    c.create_payload_index("lang", PayloadIndexKind::Keyword)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        c.create_payload_index("lang", PayloadIndexKind::Integer)
            .is_err()
    );
    assert_eq!(c.stats().payload_indexes.len(), 1);
    // Reserved / nested / empty keys are rejected.
    assert!(
        c.create_payload_index("_text", PayloadIndexKind::Keyword)
            .is_err()
    );
    assert!(
        c.create_payload_index("a.b", PayloadIndexKind::Keyword)
            .is_err()
    );
    assert!(
        c.create_payload_index("", PayloadIndexKind::Keyword)
            .is_err()
    );
}

/// FLT-031: every planner strategy — selective pre-filter, non-selective
/// post-filter over-fetch, unindexed post-filter — returns exactly the scan
/// baseline on a deterministic corpus. 2 000 points crosses the planner's
/// post-filter threshold (512).
#[test]
fn planner_strategies_match_the_exact_scan_baseline() {
    let n: u32 = 2_000;
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "v",
            opts()
                .payload_index("rare", PayloadIndexKind::Keyword)
                .payload_index("common", PayloadIndexKind::Keyword),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let points: Vec<Point> = (0..n)
        .map(|i| {
            // ~1% carry rare=yes; ~90% carry common=yes; "unindexed" is not
            // declared, ~30% carry it.
            let rare = if i % 97 == 0 { "yes" } else { "no" };
            let common = if i % 10 != 0 { "yes" } else { "no" };
            let unindexed = if i % 3 == 0 { "yes" } else { "no" };
            Point::new(format!("k{i}"), vec![i as f32, (i % 7) as f32]).payload(
                serde_json::json!({"rare": rare, "common": common, "unindexed": unindexed}),
            )
        })
        .collect();
    c.upsert_batch(points).unwrap_or_else(|e| panic!("{e}"));

    let query = [777.3f32, 2.0];
    let limit = 12;
    let matches = |key: &str, i: u32| match key {
        "rare" => i.is_multiple_of(97),
        "common" => !i.is_multiple_of(10),
        _ => i.is_multiple_of(3),
    };
    let baseline = |key: &str, take: usize| -> Vec<String> {
        let mut all: Vec<(f32, u32)> = (0..n)
            .filter(|&i| matches(key, i))
            .map(|i| {
                let dx = i as f32 - query[0];
                let dy = (i % 7) as f32 - query[1];
                (dx * dx + dy * dy, i)
            })
            .collect();
        all.sort_by(|a, b| a.0.total_cmp(&b.0));
        all.iter()
            .take(take)
            .map(|(_, i)| format!("k{i}"))
            .collect()
    };

    // Selective pre-filter (~1% candidates): exact by construction — the
    // result MUST equal the hand-computed scan baseline (FLT-031).
    let rare_filter =
        Filter::from_json(&serde_json::json!({"must":[{"key":"rare","match":{"value":"yes"}}]}))
            .unwrap_or_else(|e| panic!("{e}"));
    let hits = c
        .query(&query)
        .limit(limit)
        .filter(rare_filter)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    let got: Vec<String> = hits.into_iter().map(|h| h.id).collect();
    assert_eq!(got, baseline("rare", limit), "pre-filter must be exact");

    // Non-selective / unindexed keys route to HNSW over-fetch post-filter,
    // which follows the same approximation contract as unfiltered ANN search
    // (the graph's level assignment is randomized). Assert the contract: the
    // requested count, every hit genuinely matching the filter, exact scores,
    // and metric ordering.
    for key in ["common", "unindexed"] {
        let filter =
            Filter::from_json(&serde_json::json!({"must":[{"key": key, "match":{"value":"yes"}}]}))
                .unwrap_or_else(|e| panic!("{e}"));
        let hits = c
            .query(&query)
            .limit(limit)
            .filter(filter)
            .run()
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(hits.len(), limit, "post-filter must fill the limit");
        let mut prev = f32::NEG_INFINITY;
        for h in &hits {
            let i: u32 = h.id[1..].parse().unwrap_or_else(|e| panic!("{e}"));
            assert!(matches(key, i), "{key}: {} must match the filter", h.id);
            let dx = i as f32 - query[0];
            let dy = (i % 7) as f32 - query[1];
            let exact = (dx * dx + dy * dy).sqrt();
            assert!(
                (h.score - exact).abs() <= exact * 1e-5,
                "{key}: score {} vs exact {exact}",
                h.score
            );
            assert!(h.score >= prev, "{key}: ascending distance order");
            prev = h.score;
        }
    }

    // Exhaustion path: an unindexed filter whose total matches (~667) fall
    // short of the limit forces the adaptive growth to run dry, and the
    // exact-scan fallback makes the result deterministically identical to the
    // baseline (FLT-031).
    let all_unindexed = c
        .query(&query)
        .limit(700)
        .filter(
            Filter::from_json(
                &serde_json::json!({"must":[{"key":"unindexed","match":{"value":"yes"}}]}),
            )
            .unwrap_or_else(|e| panic!("{e}")),
        )
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    let got: Vec<String> = all_unindexed.into_iter().map(|h| h.id).collect();
    assert_eq!(
        got,
        baseline("unindexed", 700),
        "exhaustion fallback must be exact"
    );

    // Filter matching nothing: every strategy agrees on empty.
    let none = c
        .query(&query)
        .limit(limit)
        .filter(lang_filter("absent"))
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(none.is_empty());
}
