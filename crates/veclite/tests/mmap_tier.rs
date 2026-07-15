//! mmap tier integration (SPEC-002 STG-004/063/064, ADR-0004): forced-mmap
//! opens serve identical results to materialized opens, in both the indexed
//! (HNSW-from-mmap) and the over-budget (exact brute-force) tiers; writes
//! overlay correctly; checkpoint carry-forward and vacuum stay consistent.

#![cfg(not(target_arch = "wasm32"))]

use veclite::{CollectionOptions, Metric, OpenOptions, Point, Quantization, VecLite};

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "veclite-mmaptier-{}-{name}.veclite",
        std::process::id()
    ))
}

fn cleanup(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let _ = std::fs::remove_file(std::path::PathBuf::from(wal));
}

/// Build a small file-backed collection: 64 vectors on a 4-D grid + payloads.
fn build(path: &std::path::Path, n: u32) {
    cleanup(path);
    let db = VecLite::open(path).unwrap_or_else(|e| panic!("{e}"));
    let c = db
        .create_collection(
            "v",
            CollectionOptions::new(4, Metric::Euclidean).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let points: Vec<Point> = (0..n)
        .map(|i| {
            Point::new(format!("k{i}"), vec![i as f32, 0.0, 0.0, 0.0])
                .payload(serde_json::json!({"i": i}))
        })
        .collect();
    c.upsert_batch(points).unwrap_or_else(|e| panic!("{e}"));
    // db drops here: close-time checkpoint seals the segments.
}

/// Search results (ids in order) for a query under the given open options.
fn search_ids(path: &std::path::Path, opts: OpenOptions, q: &[f32], limit: usize) -> Vec<String> {
    let db = VecLite::open_with(path, opts).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    let hits = c.search(q, limit).unwrap_or_else(|e| panic!("{e}"));
    hits.into_iter().map(|h| h.id).collect()
}

#[test]
fn forced_mmap_matches_materialized_results_both_tiers() {
    let path = tmp("parity");
    build(&path, 64);
    let q = [31.4, 0.0, 0.0, 0.0];

    let ram = search_ids(&path, OpenOptions::new().mmap(false), &q, 5);
    // Indexed tier: mmap on, vectors well under the default budget.
    let mapped = search_ids(&path, OpenOptions::new().mmap(true), &q, 5);
    // Over-budget tier: budget 0 forces the exact-scan path (STG-064).
    let scanned = search_ids(&path, OpenOptions::new().mmap(true).memory_budget(0), &q, 5);

    assert_eq!(ram, vec!["k31", "k32", "k30", "k33", "k29"]);
    assert_eq!(mapped, ram);
    assert_eq!(scanned, ram);
    cleanup(&path);
}

#[test]
fn mmap_tier_serves_get_scroll_payload_and_vector() {
    let path = tmp("projection");
    build(&path, 16);
    let db = VecLite::open_with(&path, OpenOptions::new().mmap(true).memory_budget(0))
        .unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 16);

    let p = c
        .get("k7")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("k7 missing"));
    assert_eq!(p.vector, vec![7.0, 0.0, 0.0, 0.0]);
    assert_eq!(p.payload, Some(serde_json::json!({"i": 7})));

    let page = c.scroll(None, 4, None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(page.points.len(), 4);
    assert_eq!(page.points[0].id, "k0");
    assert_eq!(page.points[3].vector, vec![3.0, 0.0, 0.0, 0.0]);
    cleanup(&path);
}

#[test]
fn writes_overlay_the_mmap_base_and_persist() {
    let path = tmp("overlay");
    build(&path, 32);
    {
        let db = VecLite::open_with(&path, OpenOptions::new().mmap(true).memory_budget(0))
            .unwrap_or_else(|e| panic!("{e}"));
        let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
        // Overlay: new id, replacement of a base id, deletion of a base id.
        c.upsert(Point::new("new", vec![100.0, 0.0, 0.0, 0.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("k3", vec![200.0, 0.0, 0.0, 0.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(c.delete("k5").unwrap_or_else(|e| panic!("{e}")));
        assert_eq!(c.len(), 32); // 32 - deleted + new

        let top = c
            .search(&[100.2, 0.0, 0.0, 0.0], 1)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(top[0].id, "new");
        let repl = c
            .search(&[200.0, 0.0, 0.0, 0.0], 1)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(repl[0].id, "k3");
        let near5 = c
            .search(&[5.0, 0.0, 0.0, 0.0], 1)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_ne!(near5[0].id, "k5"); // deleted base id is gone
        // close: checkpoint seals base + overlay into a fresh compacted state
    }
    let db = VecLite::open_with(&path, OpenOptions::new().mmap(true).memory_budget(0))
        .unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 32);
    assert!(c.get("k5").unwrap_or_else(|e| panic!("{e}")).is_none());
    let got = c
        .get("k3")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("k3 missing"));
    assert_eq!(got.vector, vec![200.0, 0.0, 0.0, 0.0]);
    cleanup(&path);
}

#[test]
fn clean_checkpoint_carries_forward_and_reopens() {
    let path = tmp("carry");
    build(&path, 32);
    {
        let db = VecLite::open_with(&path, OpenOptions::new().mmap(true))
            .unwrap_or_else(|e| panic!("{e}"));
        // No mutations: this checkpoint must carry the collection forward by
        // segment reference (ADR-0004) — and the file must stay valid.
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    }
    let ids = search_ids(
        &path,
        OpenOptions::new().mmap(true),
        &[10.4, 0.0, 0.0, 0.0],
        3,
    );
    assert_eq!(ids, vec!["k10", "k11", "k9"]);
    cleanup(&path);
}

#[test]
fn vacuum_rebases_shrinks_and_stays_consistent() {
    let path = tmp("vacuum");
    build(&path, 48);
    let before = std::fs::metadata(&path)
        .unwrap_or_else(|e| panic!("{e}"))
        .len();
    {
        let db = VecLite::open_with(&path, OpenOptions::new().mmap(true).memory_budget(0))
            .unwrap_or_else(|e| panic!("{e}"));
        let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
        let dead: Vec<String> = (8..48).map(|i| format!("k{i}")).collect();
        let refs: Vec<&str> = dead.iter().map(String::as_str).collect();
        assert_eq!(c.delete_batch(&refs).unwrap_or_else(|e| panic!("{e}")), 40);
        // Windows-critical (STG-071): vacuum swaps the file while this same
        // process held it mmap'd — compact drops the map before the rename.
        db.vacuum().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.len(), 8);
        let top = c
            .search(&[6.9, 0.0, 0.0, 0.0], 2)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(top[0].id, "k7");
    }
    let after = std::fs::metadata(&path)
        .unwrap_or_else(|e| panic!("{e}"))
        .len();
    assert!(after < before, "vacuum must shrink: {before} -> {after}");
    let ids = search_ids(
        &path,
        OpenOptions::new().mmap(true),
        &[0.1, 0.0, 0.0, 0.0],
        2,
    );
    assert_eq!(ids, vec!["k0", "k1"]);
    cleanup(&path);
}

#[test]
fn read_only_serves_the_mmap_tier() {
    let path = tmp("ro");
    build(&path, 24);
    let db = VecLite::open_with(
        &path,
        OpenOptions::new()
            .read_only(true)
            .mmap(true)
            .memory_budget(0),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    let hits = c
        .search(&[12.2, 0.0, 0.0, 0.0], 2)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "k12");
    assert!(matches!(
        c.upsert(Point::new("x", vec![0.0; 4])),
        Err(veclite::VecLiteError::ReadOnly)
    ));
    cleanup(&path);
}
