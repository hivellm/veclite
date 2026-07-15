//! Registry-op crash recovery (SPEC-003 §6) and alias/rename bookkeeping:
//! rename, drop, and alias WAL entries replay exactly on reopen; live alias
//! maintenance re-points on rename and unregisters on delete; the WAL-size
//! trigger and auto-vacuum escalation fire.

#![cfg(not(target_arch = "wasm32"))]

use veclite::{CollectionOptions, Metric, OpenOptions, Point, Quantization, VecLite};

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "veclite-recovreg-{}-{name}.veclite",
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

#[test]
fn crash_replays_rename_drop_and_alias_ops() {
    let path = tmp("registry");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let a = db
            .create_collection("alpha", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        a.upsert(Point::new("x", vec![1.0, 0.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_collection("doomed", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        // Checkpoint the baseline, then journal registry ops that only the
        // WAL knows about, then crash.
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        db.rename_collection("alpha", "beta")
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_alias("current", "beta")
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_alias("stale", "beta")
            .unwrap_or_else(|e| panic!("{e}"));
        db.delete_alias("stale").unwrap_or_else(|e| panic!("{e}"));
        db.delete_collection("doomed")
            .unwrap_or_else(|e| panic!("{e}"));
        db.__test_simulate_crash(); // WAL survives for replay
    }
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    // Rename replayed: old name gone, new name serves the data.
    assert!(db.collection("alpha").is_err());
    let beta = db.collection("beta").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(beta.len(), 1);
    // Alias create + delete replayed.
    let via_alias = db.collection("current").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(via_alias.len(), 1);
    assert!(db.collection("stale").is_err());
    assert_eq!(
        db.aliases(),
        vec![("current".to_owned(), "beta".to_owned())]
    );
    // Drop replayed.
    assert!(db.collection("doomed").is_err());
    cleanup(&path);
}

#[test]
fn rename_repoints_aliases_and_delete_unregisters_them() {
    let db = VecLite::memory();
    db.create_collection("v1", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    db.create_alias("live", "v1")
        .unwrap_or_else(|e| panic!("{e}"));

    db.rename_collection("v1", "v2")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(db.aliases(), vec![("live".to_owned(), "v2".to_owned())]);
    db.collection("live")
        .unwrap_or_else(|e| panic!("{e}"))
        .upsert(Point::new("x", vec![0.0, 0.0]))
        .unwrap_or_else(|e| panic!("{e}"));

    // Deleting the collection drops its aliases too.
    db.delete_collection("v2").unwrap_or_else(|e| panic!("{e}"));
    assert!(db.collection("live").is_err());
    assert!(db.aliases().is_empty());

    // Alias error paths: unknown target, duplicate alias name, unknown delete.
    db.create_collection("c", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(db.create_alias("a", "ghost").is_err());
    db.create_alias("a", "c").unwrap_or_else(|e| panic!("{e}"));
    assert!(db.create_alias("a", "c").is_err());
    assert!(db.delete_alias("ghost-alias").is_err());
}

#[test]
fn wal_size_trigger_checkpoints_and_auto_vacuum_escalates() {
    let path = tmp("triggers");
    cleanup(&path);
    {
        // Tiny WAL limit: every batch crosses it and drives a checkpoint on
        // the write path (WAL-030a); aggressive threshold escalates the next
        // checkpoint to a vacuum once deletes accumulate (STG-072).
        let db = VecLite::open_with(
            &path,
            OpenOptions::new()
                .wal_size_limit(512)
                .auto_vacuum_threshold(0.1),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("v", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        let points: Vec<Point> = (0..64)
            .map(|i| Point::new(format!("k{i}"), vec![i as f32, 0.0]))
            .collect();
        c.upsert_batch(points).unwrap_or_else(|e| panic!("{e}"));
        let dead: Vec<String> = (0..32).map(|i| format!("k{i}")).collect();
        let refs: Vec<&str> = dead.iter().map(String::as_str).collect();
        assert_eq!(c.delete_batch(&refs).unwrap_or_else(|e| panic!("{e}")), 32);
        // Another write crosses the tiny WAL limit again → checkpoint sees a
        // 33% tombstone ratio → vacuum resets it.
        c.upsert(Point::new("tail", vec![99.0, 0.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.len(), 33);
        let hits = c.search(&[99.0, 0.0], 1).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(hits[0].id, "tail");
    }
    // Everything durable after triggers: reopen and verify.
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 33);
    assert!(c.get("k5").unwrap_or_else(|e| panic!("{e}")).is_none());
    cleanup(&path);
}

#[test]
fn stats_query_builder_and_search_batch_surfaces() {
    let db = VecLite::memory();
    let c = db
        .create_collection("s", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert_batch(vec![
        Point::new("a", vec![0.0, 0.0]).payload(serde_json::json!({"t": 1})),
        Point::new("b", vec![1.0, 0.0]),
    ])
    .unwrap_or_else(|e| panic!("{e}"));
    c.delete("b").unwrap_or_else(|e| panic!("{e}"));

    let stats = c.stats();
    assert_eq!(stats.len, 1);
    assert_eq!(stats.tombstones, 1);
    assert_eq!(stats.dimension, 2);
    assert!(!stats.auto_embed);

    // Query builder: vector projection + ef_search override + error paths.
    let hits = c
        .query(&[0.0, 0.0])
        .limit(1)
        .ef_search(64)
        .with_payload(true)
        .with_vector(true)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].vector.as_deref(), Some(&[0.0f32, 0.0][..]));
    assert_eq!(hits[0].payload, Some(serde_json::json!({"t": 1})));
    assert!(c.query(&[0.0, 0.0]).limit(0).run().is_err());
    assert!(c.query(&[0.0, 0.0]).limit(1).ef_search(0).run().is_err());
    assert!(c.query(&[f32::NAN, 0.0]).limit(1).run().is_err());
    assert!(c.query(&[0.0]).limit(1).run().is_err());

    // Parallel batch search: per-query results, errors isolated per query.
    let results = c.search_batch(&[vec![0.0, 0.0], vec![9.0, 9.0]], 1);
    assert_eq!(results.len(), 2);
    for r in results {
        assert_eq!(r.unwrap_or_else(|e| panic!("{e}"))[0].id, "a");
    }
}

#[test]
fn dot_product_collection_serves_exact_search() {
    // DotProduct builds no HNSW graph (ADR-0002): the brute-force path serves
    // it on every target; scores are raw dot products, descending.
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "dp",
            CollectionOptions::new(2, Metric::DotProduct).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert_batch(vec![
        Point::new("small", vec![0.1, 0.1]),
        Point::new("big", vec![10.0, 10.0]),
    ])
    .unwrap_or_else(|e| panic!("{e}"));
    let hits = c.search(&[1.0, 1.0], 2).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "big");
    assert!(hits[0].score > hits[1].score);
    c.reindex().unwrap_or_else(|e| panic!("{e}")); // stays graph-less, still exact
    let hits = c.search(&[1.0, 1.0], 1).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "big");
}
