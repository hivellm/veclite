//! Format v1 golden-file compatibility (SPEC-002 frozen-normative, PRD NFR-11).
//! A committed v1 `.veclite` file MUST keep opening and returning its recorded
//! query results on every future commit — this is the enforceable half of the
//! format freeze that gate G2 establishes.
//!
//! `golden_v1_opens_and_returns_recorded_results` is the guard (runs on every
//! `cargo test`). `regenerate_golden_v1` is `#[ignore]`d and only run by hand
//! (`cargo test --test golden regenerate -- --ignored`) when the format is
//! intentionally revised — which, post-freeze, requires a new format version.

use std::path::PathBuf;

use veclite::{CollectionOptions, Metric, Point, Quantization, VecLite};

/// Path to the committed golden file, relative to the crate manifest.
fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("compat")
        .join("golden")
        .join("v1.veclite")
}

fn docs_opts() -> CollectionOptions {
    CollectionOptions::new(3, Metric::Euclidean).quantization(Quantization::None)
}

fn vecs_opts() -> CollectionOptions {
    CollectionOptions::new(2, Metric::Cosine).quantization(Quantization::None)
}

/// The single source of truth for the golden dataset, used by both the
/// regenerator and the guard so they can never drift.
fn populate(db: &VecLite) {
    let docs = db
        .create_collection("docs", docs_opts())
        .unwrap_or_else(|e| panic!("{e}"));
    docs.upsert(Point::new("a", vec![1.0, 0.0, 0.0]).payload(serde_json::json!({"lang": "en"})))
        .unwrap_or_else(|e| panic!("{e}"));
    docs.upsert(Point::new("b", vec![0.0, 1.0, 0.0]).payload(serde_json::json!({"lang": "fr"})))
        .unwrap_or_else(|e| panic!("{e}"));
    docs.upsert(Point::new("c", vec![0.0, 0.0, 1.0]).payload(serde_json::json!({"lang": "de"})))
        .unwrap_or_else(|e| panic!("{e}"));

    let vecs = db
        .create_collection("vecs", vecs_opts())
        .unwrap_or_else(|e| panic!("{e}"));
    vecs.upsert(Point::new("p", vec![3.0, 4.0]))
        .unwrap_or_else(|e| panic!("{e}"));
    vecs.upsert(Point::new("q", vec![1.0, 0.0]))
        .unwrap_or_else(|e| panic!("{e}"));
}

/// Assert the recorded query results. Shared by the guard and (as a sanity
/// check) the regenerator.
fn assert_recorded_results(db: &VecLite) {
    assert_eq!(db.list_collections(), vec!["docs", "vecs"]);

    let docs = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(docs.len(), 3);
    let b = docs
        .get("b")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("b missing"));
    assert_eq!(b.vector, vec![0.0, 1.0, 0.0]);
    assert_eq!(b.payload, Some(serde_json::json!({"lang": "fr"})));
    // Nearest to a point closest to `a`.
    let near_a = docs
        .search(&[0.9, 0.1, 0.0], 1)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(near_a[0].id, "a");

    let vecs = db.collection("vecs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(vecs.len(), 2);
    // Cosine: [1,1] is closer in angle to p=[3,4] than to q=[1,0].
    let near_p = vecs
        .search(&[1.0, 1.0], 1)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(near_p[0].id, "p");
}

/// GUARD (runs every `cargo test`): copy the committed golden file to a temp
/// dir (so the repo tree is never mutated — a read would still create a WAL
/// sidecar), open it, and assert the recorded results still hold.
#[test]
fn golden_v1_opens_and_returns_recorded_results() {
    let src = golden_path();
    assert!(
        src.exists(),
        "missing golden file {src:?}; regenerate with `cargo test --test golden regenerate -- --ignored`"
    );
    let scratch =
        std::env::temp_dir().join(format!("veclite-golden-{}.veclite", std::process::id()));
    let _ = std::fs::remove_file(&scratch);
    std::fs::copy(&src, &scratch).unwrap_or_else(|e| panic!("{e}"));

    let db = VecLite::open(&scratch).unwrap_or_else(|e| panic!("golden v1 failed to open: {e}"));
    assert_recorded_results(&db);
    drop(db);

    let _ = std::fs::remove_file(&scratch);
    let mut wal = scratch.into_os_string();
    wal.push("-wal");
    let _ = std::fs::remove_file(wal);
}

/// REGENERATOR (`#[ignore]`, run by hand only): rewrite the committed golden
/// file. Post-freeze this is a deliberate, reviewed act — any format change is a
/// new version, not a silent golden rewrite.
#[test]
#[ignore = "regenerates the committed golden fixture; run explicitly"]
fn regenerate_golden_v1() {
    let path = golden_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{e}"));
    }
    let _ = std::fs::remove_file(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        populate(&db);
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    } // clean close: the WAL is truncated to its header

    // Drop the sidecar so only the self-contained main file is committed.
    let mut wal = path.clone().into_os_string();
    wal.push("-wal");
    let _ = std::fs::remove_file(wal);

    // Sanity: what we just wrote reads back with the recorded results.
    let scratch = std::env::temp_dir().join(format!(
        "veclite-golden-regen-{}.veclite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&scratch);
    std::fs::copy(&path, &scratch).unwrap_or_else(|e| panic!("{e}"));
    let db = VecLite::open(&scratch).unwrap_or_else(|e| panic!("{e}"));
    assert_recorded_results(&db);
    drop(db);
    let _ = std::fs::remove_file(&scratch);
    let mut swal = scratch.into_os_string();
    swal.push("-wal");
    let _ = std::fs::remove_file(swal);
}
