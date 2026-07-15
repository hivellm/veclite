//! Sparse-lane persistence (SPEC-007 HYB-030/031) and the auto-embed `.text()`
//! hybrid lane (HYB-011): the BYO sparse lane survives checkpoint+reopen and a
//! kill-9 crash; vacuum drops tombstoned postings; one query string fills both
//! lanes on an auto-embed collection.

#![cfg(not(target_arch = "wasm32"))]

use veclite::{
    CollectionOptions, Metric, Point, Quantization, SparseVector, VecLite, VecLiteError,
};

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "veclite-hybperse-{}-{name}.veclite",
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

fn sv(indices: Vec<u32>, values: Vec<f32>) -> SparseVector {
    SparseVector { indices, values }
}

/// BYO points: dense grid position + a sparse lane keyed by id parity.
fn byo_points() -> Vec<Point> {
    (0..8u32)
        .map(|i| {
            Point::new(format!("k{i}"), vec![i as f32, 0.0])
                .sparse(sv(vec![i, 100], vec![1.0 + i as f32, 0.5]))
        })
        .collect()
}

#[test]
fn byo_sparse_survives_checkpoint_and_reopen() {
    let path = tmp("reopen");
    let query = sv(vec![3, 100], vec![2.0, 1.0]);
    let (before_sparse, before_hybrid) = {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("v", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert_batch(byo_points())
            .unwrap_or_else(|e| panic!("{e}"));
        let s: Vec<String> = c
            .search_sparse(&query, 10)
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .map(|h| h.id)
            .collect();
        let h: Vec<String> = c
            .hybrid_query()
            .dense(&[3.0, 0.0])
            .sparse(&query)
            .limit(5)
            .run()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .map(|hit| hit.id)
            .collect();
        (s, h)
        // db drops: checkpoint seals the SPARSE segment
    };
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    // The sparse lane came back from the SPARSE segment, not the WAL.
    let after_sparse: Vec<String> = c
        .search_sparse(&query, 10)
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .map(|h| h.id)
        .collect();
    let after_hybrid: Vec<String> = c
        .hybrid_query()
        .dense(&[3.0, 0.0])
        .sparse(&query)
        .limit(5)
        .run()
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .map(|hit| hit.id)
        .collect();
    assert_eq!(
        before_sparse, after_sparse,
        "sparse ranking must survive reopen"
    );
    assert!(!after_sparse.is_empty());
    assert_eq!(
        before_hybrid, after_hybrid,
        "hybrid ranking must survive reopen"
    );
    // The reconstructed sparse vector round-trips exactly (HYB-001 order held).
    let got = c
        .get("k3")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("k3"));
    assert_eq!(got.sparse, Some(sv(vec![3, 100], vec![4.0, 0.5])));
    cleanup(&path);
}

#[test]
fn sparse_lane_recovers_exactly_after_a_crash() {
    let path = tmp("crash");
    let query = sv(vec![5, 100], vec![3.0, 1.0]);
    let before = {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("v", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert_batch(byo_points())
            .unwrap_or_else(|e| panic!("{e}"));
        // Checkpoint the first 8, then add + delete uncheckpointed so recovery
        // must merge the sealed SPARSE with WAL deltas (HYB-030).
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("k9", vec![9.0, 0.0]).sparse(sv(vec![9, 100], vec![10.0, 0.5])))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(c.delete("k2").unwrap_or_else(|e| panic!("{e}")));
        let ranked: Vec<(String, String)> = c
            .search_sparse(&query, 10)
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .map(|h| (h.id, format!("{:.4}", h.score)))
            .collect();
        db.__test_simulate_crash();
        ranked
    };
    // Recovered (sealed SPARSE + WAL replay) must equal the pre-crash ranking.
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    let recovered: Vec<(String, String)> = c
        .search_sparse(&query, 10)
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .map(|h| (h.id, format!("{:.4}", h.score)))
        .collect();
    assert_eq!(before, recovered);
    assert!(c.get("k2").unwrap_or_else(|e| panic!("{e}")).is_none());
    cleanup(&path);
}

#[test]
fn vacuum_drops_tombstoned_sparse_postings() {
    let path = tmp("vacuum");
    {
        let db = VecLite::open_with(
            &path,
            veclite::OpenOptions::new().auto_vacuum_threshold(0.0),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("v", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert_batch(byo_points())
            .unwrap_or_else(|e| panic!("{e}"));
        // Delete the even ids, vacuum, and confirm only odd ids remain scored.
        let dead: Vec<&str> = ["k0", "k2", "k4", "k6"].to_vec();
        assert_eq!(c.delete_batch(&dead).unwrap_or_else(|e| panic!("{e}")), 4);
        db.vacuum().unwrap_or_else(|e| panic!("{e}"));
        // Query index 100 (shared by all) — every survivor scores, no ghosts.
        let ids: Vec<String> = c
            .search_sparse(&sv(vec![100], vec![1.0]), 10)
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .map(|h| h.id)
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["k1", "k3", "k5", "k7"]);
    }
    // Reopen: the vacuumed SPARSE segment holds only the survivors.
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    let mut ids: Vec<String> = c
        .search_sparse(&sv(vec![100], vec![1.0]), 10)
        .unwrap_or_else(|e| panic!("{e}"))
        .into_iter()
        .map(|h| h.id)
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["k1", "k3", "k5", "k7"]);
    cleanup(&path);
}

#[test]
fn auto_embed_text_lane_fills_both_lanes() {
    let db = VecLite::memory();
    let c = db
        .create_collection("docs", CollectionOptions::auto_embed("bm25", 64))
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert_text_batch(
        [
            ("cats", "cats are small furry animals that meow"),
            ("dogs", "dogs are loyal furry animals that bark"),
            ("cars", "cars are fast vehicles with engines"),
        ]
        .into_iter()
        .map(|(i, t)| (i.to_owned(), t.to_owned(), None))
        .collect(),
    )
    .unwrap_or_else(|e| panic!("{e}"));

    // One string drives both lanes (HYB-011): the furry-animal doc wins.
    let hits = c
        .hybrid_query()
        .text("furry animals that meow")
        .limit(3)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "cats");
    assert!(hits.iter().any(|h| h.id == "dogs"));

    // The auto-maintained sparse lane is exposed on the stored docs (HYB-002a).
    let cats = c
        .get("cats")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("cats"));
    assert!(cats.sparse.is_some(), "auto-embed maintains a sparse lane");

    // `.text()` on a BYO collection is a mode error.
    let byo = db
        .create_collection("byo", opts())
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        byo.hybrid_query().text("q").limit(1).run(),
        Err(VecLiteError::InvalidArgument(_))
    ));
}
