//! VecLite Rust quickstart (SPEC-004). This file is the executed docs sample
//! (`docs/src/quickstart/rust.md` includes it verbatim) and the `cargo xtask
//! docs` runner runs it, so the sample can never go stale (REL-041). It
//! exercises the core flow end to end and exits non-zero on any surprise.
//!
//! Run: `cargo run -p hivellm-veclite --example quickstart`

use serde_json::json;
use veclite::{CollectionOptions, Condition, Filter, Metric, Point, VecLite};

fn main() -> veclite::Result<()> {
    // A durable single-file database — no server, no config (FR-01/02). Use a
    // temp path so the example is self-contained and repeatable.
    let dir = std::env::temp_dir().join(format!("veclite-quickstart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(veclite::VecLiteError::Io)?;
    let db = VecLite::open(dir.join("app.veclite"))?;

    // BYO-vector collection, cosine metric, full-precision.
    let docs = db.create_collection("docs", CollectionOptions::new(3, Metric::Cosine))?;
    docs.upsert(Point::new("a", vec![1.0, 0.0, 0.0]).payload(json!({ "lang": "en" })))?;
    docs.upsert(Point::new("b", vec![0.0, 1.0, 0.0]).payload(json!({ "lang": "fr" })))?;
    docs.upsert(Point::new("c", vec![0.9, 0.1, 0.0]).payload(json!({ "lang": "en" })))?;

    // k-NN search with a payload filter (SPEC-006): only the English vectors.
    let hits = docs
        .query(&[1.0, 0.0, 0.0])
        .filter(Filter::new().must(Condition::eq("lang", "en")))
        .limit(2)
        .run()?;
    let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(ids, ["a", "c"], "filtered nearest neighbours");

    // An auto-embed (BM25) collection: text in, ranked ids out (SPEC-005). No
    // model download, no network — a pure-Rust sparse embedder.
    let notes = db.create_collection("notes", CollectionOptions::auto_embed("bm25", 128))?;
    notes.upsert_text("n1", "the quick brown fox")?;
    notes.upsert_text("n2", "a lazy sleeping dog")?;
    assert!(!notes.search_text("quick fox", 2)?.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
    println!(
        "veclite {}: quickstart OK ({ids:?})",
        env!("CARGO_PKG_VERSION")
    );
    Ok(())
}
