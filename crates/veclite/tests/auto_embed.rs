//! Auto-embed collections (SPEC-005 §4–5): `upsert_text`/`search_text` with the
//! built-in BM25 provider, fail-fast rules (EMB-021), and reopen determinism
//! (EMB-020).

use serde_json::json;
use veclite::{CollectionOptions, Metric, Point, VecLite, VecLiteError};

fn ids(hits: &[veclite::Hit]) -> Vec<String> {
    hits.iter().map(|h| h.id.clone()).collect()
}

fn seed(c: &veclite::Collection) {
    c.upsert_text("cats", "cats are small furry animals that meow")
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert_text("dogs", "dogs are loyal animals that bark loudly")
        .unwrap_or_else(|e| panic!("{e}"));
    c.upsert_text("cars", "cars are fast vehicles with powerful engines")
        .unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn upsert_text_and_search_text_find_the_lexical_match() {
    let db = VecLite::memory();
    let c = db
        .create_collection("docs", CollectionOptions::auto_embed("bm25", 128))
        .unwrap_or_else(|e| panic!("{e}"));
    seed(&c);
    assert_eq!(c.len(), 3);

    // A query sharing terms with the cats document ranks it first.
    let hits = c
        .search_text("furry animals that meow", 3)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "cats");

    // A query about vehicles ranks the cars document first.
    let hits = c
        .search_text("fast vehicles with engines", 1)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "cars");
}

#[test]
fn unknown_provider_is_rejected_at_creation() {
    let db = VecLite::memory();
    let err = db.create_collection("x", CollectionOptions::auto_embed("bm52", 128));
    let Err(VecLiteError::UnsupportedProvider {
        requested,
        available,
    }) = err
    else {
        panic!("expected UnsupportedProvider");
    };
    assert_eq!(requested, "bm52");
    assert!(available.contains(&"bm25".to_owned()));
}

#[test]
fn text_ops_on_byo_collection_are_rejected() {
    let db = VecLite::memory();
    let byo = db
        .create_collection("byo", CollectionOptions::new(3, Metric::Euclidean))
        .unwrap_or_else(|e| panic!("{e}"));
    // Vector ops work.
    byo.upsert(Point::new("a", vec![1.0, 2.0, 3.0]))
        .unwrap_or_else(|e| panic!("{e}"));
    // Text ops fail fast.
    assert!(matches!(
        byo.upsert_text("b", "hello"),
        Err(VecLiteError::InvalidArgument(_))
    ));
    assert!(matches!(
        byo.search_text("hello", 1),
        Err(VecLiteError::InvalidArgument(_))
    ));
}

#[test]
fn reserved_key_in_text_payload_is_rejected() {
    let db = VecLite::memory();
    let c = db
        .create_collection("docs", CollectionOptions::auto_embed("bm25", 64))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        c.upsert_text_with("a", "some text", json!({"_text": "sneaky"})),
        Err(VecLiteError::InvalidArgument(_))
    ));
    // A normal payload alongside text is fine and searchable.
    c.upsert_text_with("b", "some text", json!({"lang": "en"}))
        .unwrap_or_else(|e| panic!("{e}"));
    let hit = c
        .search_text("some text", 1)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hit[0].id, "b");
    assert_eq!(
        hit[0]
            .payload
            .as_ref()
            .and_then(|p| p.get("lang"))
            .and_then(|v| v.as_str()),
        Some("en")
    );
}

#[test]
fn reopen_preserves_search_text_results() {
    let path =
        std::env::temp_dir().join(format!("veclite-autoembed-{}.veclite", std::process::id()));
    let mut wal = path.clone().into_os_string();
    wal.push("-wal");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&wal);

    let before = {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", CollectionOptions::auto_embed("bm25", 128))
            .unwrap_or_else(|e| panic!("{e}"));
        seed(&c);
        let b = ids(&c
            .search_text("furry animals that meow", 3)
            .unwrap_or_else(|e| panic!("{e}")));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        b
    };

    // Reopen on a fresh handle: the vocabulary rebuilds from the stored `_text`.
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 3);
    let after = ids(&c
        .search_text("furry animals that meow", 3)
        .unwrap_or_else(|e| panic!("{e}")));
    assert_eq!(before, after);

    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&wal);
}

#[test]
fn other_providers_work_via_auto_embed() {
    let db = VecLite::memory();
    for provider in ["tfidf", "bow", "char_ngram"] {
        let c = db
            .create_collection(provider, CollectionOptions::auto_embed(provider, 128))
            .unwrap_or_else(|e| panic!("{provider}: {e}"));
        seed(&c);
        let hits = c
            .search_text("loyal animals that bark", 1)
            .unwrap_or_else(|e| panic!("{provider}: {e}"));
        assert_eq!(hits[0].id, "dogs", "provider {provider}");
    }
}

#[test]
fn refit_is_explicit_and_keeps_search_working() {
    let db = VecLite::memory();
    let c = db
        .create_collection("docs", CollectionOptions::auto_embed("bm25", 64))
        .unwrap_or_else(|e| panic!("{e}"));
    seed(&c);
    c.refit().unwrap_or_else(|e| panic!("{e}"));
    let hits = c
        .search_text("loyal dogs that bark", 1)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "dogs");
}
