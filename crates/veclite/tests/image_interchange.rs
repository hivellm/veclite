//! Interchange contract for the full-image codec (WASM-010, SPEC-002): the
//! bytes `VecLite::serialize` produces are a valid `.veclite` v1 file — native
//! `open` reads them — and a native file's committed bytes load through
//! `VecLite::deserialize`. These run on native (the wasm binding exercises the
//! same `serialize`/`deserialize` in `crates/veclite-wasm`), so they double as
//! the golden check that the in-memory image writer and the file pager agree.

use std::io::Read;

use veclite::{CollectionOptions, Metric, Point, SparseVector, VecLite};

/// Build a database with a mix of features: a BYO collection with payloads,
/// sparse lanes, and an alias, plus a second collection — enough to cover
/// CONFIG/VECTORS/IDDIR/PAYLOAD/SPARSE segments and the multi-collection TOC.
fn seeded_memory() -> VecLite {
    let db = VecLite::memory();
    let docs = db
        .create_collection("docs", CollectionOptions::new(3, Metric::Cosine))
        .unwrap_or_else(|e| panic!("create docs: {e}"));
    docs.upsert(
        Point::new("a", vec![1.0, 0.0, 0.0])
            .payload(serde_json::json!({"lang": "en"}))
            .sparse(SparseVector {
                indices: vec![2, 5],
                values: vec![0.5, 1.5],
            }),
    )
    .unwrap_or_else(|e| panic!("upsert a: {e}"));
    docs.upsert(Point::new("b", vec![0.0, 1.0, 0.0]).payload(serde_json::json!({"lang": "pt"})))
        .unwrap_or_else(|e| panic!("upsert b: {e}"));
    docs.upsert(Point::new("c", vec![0.0, 0.0, 1.0]))
        .unwrap_or_else(|e| panic!("upsert c: {e}"));
    db.create_alias("documents", "docs")
        .unwrap_or_else(|e| panic!("alias: {e}"));

    let notes = db
        .create_collection("notes", CollectionOptions::new(2, Metric::Euclidean))
        .unwrap_or_else(|e| panic!("create notes: {e}"));
    notes
        .upsert(Point::new("n1", vec![3.0, 4.0]))
        .unwrap_or_else(|e| panic!("upsert n1: {e}"));
    db
}

/// The ranked ids for a query, for comparing two databases' behavior.
fn search_ids(db: &VecLite, coll: &str, query: &[f32], limit: usize) -> Vec<String> {
    let c = db
        .collection(coll)
        .unwrap_or_else(|e| panic!("collection {coll}: {e}"));
    c.query(query)
        .limit(limit)
        .run()
        .unwrap_or_else(|e| panic!("search: {e}"))
        .into_iter()
        .map(|h| h.id)
        .collect()
}

fn tmp_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "veclite-image-{}-{tag}.veclite",
        std::process::id()
    ))
}

#[test]
fn serialize_output_opens_with_native_pager() {
    let db = seeded_memory();
    let bytes = db.serialize().unwrap_or_else(|e| panic!("serialize: {e}"));

    // The image is a valid v1 file: write it verbatim and open it natively.
    let path = tmp_path("open");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("write: {e}"));

    {
        let opened = VecLite::open(&path).unwrap_or_else(|e| panic!("open image: {e}"));
        let mut colls = opened.list_collections();
        colls.sort();
        assert_eq!(colls, vec!["docs".to_owned(), "notes".to_owned()]);

        // Payloads, aliases, and vectors survived the round-trip.
        let via_alias = opened
            .collection("documents")
            .unwrap_or_else(|e| panic!("alias resolve: {e}"));
        assert_eq!(via_alias.len(), 3);
        let a = via_alias
            .get("a")
            .unwrap_or_else(|e| panic!("get a: {e}"))
            .unwrap_or_else(|| panic!("a missing"));
        assert_eq!(a.payload, Some(serde_json::json!({"lang": "en"})));
        assert_eq!(a.vector, vec![1.0, 0.0, 0.0]);

        assert_eq!(
            search_ids(&opened, "docs", &[0.9, 0.1, 0.0], 1),
            vec!["a".to_owned()]
        );
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn native_file_bytes_deserialize() {
    // Create a real file-backed database, then read its committed bytes and
    // load them through the in-memory image codec.
    let path = tmp_path("native");
    let _ = std::fs::remove_file(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("open: {e}"));
        let docs = db
            .create_collection("docs", CollectionOptions::new(3, Metric::Cosine))
            .unwrap_or_else(|e| panic!("create: {e}"));
        docs.upsert(Point::new("a", vec![1.0, 0.0, 0.0]).payload(serde_json::json!({"k": 1})))
            .unwrap_or_else(|e| panic!("upsert: {e}"));
        docs.upsert(Point::new("b", vec![0.0, 1.0, 0.0]))
            .unwrap_or_else(|e| panic!("upsert: {e}"));
        db.checkpoint()
            .unwrap_or_else(|e| panic!("checkpoint: {e}"));
    } // drop releases the lock

    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("reopen: {e}"))
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("read: {e}"));

    let loaded = VecLite::deserialize(&bytes).unwrap_or_else(|e| panic!("deserialize: {e}"));
    assert_eq!(loaded.list_collections(), vec!["docs".to_owned()]);
    let c = loaded
        .collection("docs")
        .unwrap_or_else(|e| panic!("collection: {e}"));
    assert_eq!(c.len(), 2);
    let a = c
        .get("a")
        .unwrap_or_else(|e| panic!("get: {e}"))
        .unwrap_or_else(|| panic!("a missing"));
    assert_eq!(a.payload, Some(serde_json::json!({"k": 1})));
    assert_eq!(
        search_ids(&loaded, "docs", &[0.9, 0.1, 0.0], 1),
        vec!["a".to_owned()]
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serialize_deserialize_round_trip_preserves_behavior() {
    let db = seeded_memory();
    let bytes = db.serialize().unwrap_or_else(|e| panic!("serialize: {e}"));
    let back = VecLite::deserialize(&bytes).unwrap_or_else(|e| panic!("deserialize: {e}"));

    let mut a = db.list_collections();
    let mut b = back.list_collections();
    a.sort();
    b.sort();
    assert_eq!(a, b);

    // Identical ranked ids on both collections.
    assert_eq!(
        search_ids(&db, "docs", &[0.2, 0.9, 0.1], 3),
        search_ids(&back, "docs", &[0.2, 0.9, 0.1], 3)
    );
    assert_eq!(
        search_ids(&db, "notes", &[3.0, 4.0], 1),
        search_ids(&back, "notes", &[3.0, 4.0], 1)
    );

    // The sparse lane survived: a hybrid query over term 5 still finds "a".
    let c = back
        .collection("docs")
        .unwrap_or_else(|e| panic!("collection: {e}"));
    let hits = c
        .hybrid_query()
        .sparse(&SparseVector {
            indices: vec![5],
            values: vec![1.0],
        })
        .limit(3)
        .run()
        .unwrap_or_else(|e| panic!("hybrid: {e}"));
    assert!(hits.iter().any(|h| h.id == "a"), "sparse lane lost");
}

#[test]
fn empty_database_round_trips() {
    let db = VecLite::memory();
    let bytes = db.serialize().unwrap_or_else(|e| panic!("serialize: {e}"));
    let back = VecLite::deserialize(&bytes).unwrap_or_else(|e| panic!("deserialize: {e}"));
    assert!(back.list_collections().is_empty());
}
